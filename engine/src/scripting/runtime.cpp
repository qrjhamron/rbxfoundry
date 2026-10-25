// Official Luau runtime core: VM lifecycle, coroutine scheduler,
// debugger hooks, binding registration and the Runtime public API.
// All Luau C binding functions live in bindings.cpp.
#include "runtime_internal.hpp"
#include "blox/terrain/terrain.hpp"
#include "blox/gui/ui.hpp"

#include <algorithm>
#include <cmath>
#include <cstring>

namespace blox {


Runtime* rt_of(lua_State* L) {
    lua_getfield(L, LUA_REGISTRYINDEX, "blox_rt");
    Runtime* rt = static_cast<Runtime*>(lua_touserdata(L, -1));
    lua_pop(L, 1);
    return rt;
}

Thread* make_thread(Runtime* rt, lua_State* L, const std::string& owner) {
    lua_State* co = lua_newthread(L);
    luaL_sandboxthread(co);
    auto thread = std::make_unique<Thread>();
    thread->co = co;
    thread->owner = owner;
    thread->co_ref = lua_ref(L, -1); lua_pop(L, 1); // keep the thread alive
    Thread* raw = thread.get();
    rt->impl_->threads.push_back(std::move(thread));
    return raw;
}

void push_args(lua_State* L, const std::vector<LuaArg>& args); // fwd

void kill_thread(Runtime* rt, lua_State* L, Thread* t) {
    t->dead = true;
    lua_unref(L, t->co_ref);
}

// Fire a signal: spawn a thread per live connection and resume waiters.
void fire_signal(Runtime* rt, lua_State* L, SignalImpl* sig, const std::vector<LuaArg>& args) {
    if (!sig) return;
    auto waiters = sig->waiters;
    sig->waiters.clear();
    for (Thread* t : waiters) {
        if (t->dead) continue;
        t->wait = Thread::Wait::None;
        t->waiting_on = nullptr;
        t->resume_args = args;
        t->ready_step = 0;
        t->wake = rt->time();
    }
    std::vector<int> fn_refs;
    for (auto& [id, conn] : sig->conns) {
        if (conn.connected) {
            fn_refs.push_back(conn.fn_ref);
            if (conn.once) conn.connected = false;
        }
    }
    for (int ref : fn_refs) {
        Thread* t = make_thread(rt, L, rt->impl_->current_owner);
        lua_rawgeti(t->co, LUA_REGISTRYINDEX, ref);
        push_args(t->co, args);
        t->pending_args = static_cast<int>(args.size());
        t->wake = rt->time();
        t->ready_step = 0;
    }
}

// --------------------------------------------------------------- scheduler --

// ------------------------------------------------------------- debugger --

// The engine process is single-threaded and debugs one Runtime at a
// time; the VM debugstep callback reaches it through this pointer.
Runtime* g_debug_runtime = nullptr;

void debug_reenable_breakpoint(Runtime* rt, Thread* t); // fwd

void debug_break_hook(lua_State* L, lua_Debug*) {
    // LOP_BREAK re-executes the original instruction after this hook;
    // pausing is the hook's job: yield the thread and let
    // resume_thread convert the yield into the paused state. The
    // location is captured here, inside the hook, where lua_getinfo
    // still reports the breakpoint's own line (after the yield, the
    // saved pc reads as the previous instruction).
    if (g_debug_runtime) {
        lua_Debug info;
        if (lua_getinfo(L, 0, "sl", &info)) {
            g_debug_runtime->impl_->debug_pause_line = info.currentline;
            g_debug_runtime->impl_->debug_pause_script = info.source ? info.source : "";
        }
        g_debug_runtime->impl_->debug_break_hit = true;
        lua_yield(L, 0);
    }
}

void debug_step_hook(lua_State* L, lua_Debug*) {
    Runtime* rt = g_debug_runtime;
    if (!rt || !rt->impl_->debug_paused_candidate) return;
    if (rt->impl_->debug_paused_candidate->co != L) return;
    if (rt->impl_->debug_step_mode == 0) return;
    lua_Debug info;
    if (!lua_getinfo(L, 0, "sl", &info)) return;
    const int depth = lua_stackdepth(L);
    if (rt->impl_->debug_reenable_line && rt->impl_->debug_reenable_thread &&
        rt->impl_->debug_reenable_thread->co == L &&
        info.currentline != rt->impl_->debug_reenable_line) {
        debug_reenable_breakpoint(rt, rt->impl_->debug_reenable_thread);
    }
    if (rt->impl_->debug_step_mode == 4) {
        // Skip-once: run past the breakpoint instruction we just
        // resumed from, then hand control back to normal execution.
        if (info.currentline != rt->impl_->debug_step_line || depth != rt->impl_->debug_step_depth) {
            rt->impl_->debug_step_mode = 0;
            rt->impl_->debug_paused_candidate = nullptr;
            lua_singlestep(L, 0);
        }
        return;
    }
    bool hit = false;
    if (rt->impl_->debug_step_mode == 1) {
        hit = info.currentline != rt->impl_->debug_step_line;
    } else if (rt->impl_->debug_step_mode == 2) {
        hit = depth <= rt->impl_->debug_step_depth && info.currentline != rt->impl_->debug_step_line;
    } else if (rt->impl_->debug_step_mode == 3) {
        hit = depth < rt->impl_->debug_step_depth;
    }
    if (hit) {
        rt->impl_->debug_pause_line = info.currentline;
        rt->impl_->debug_pause_script = info.source ? info.source : "";
        rt->impl_->debug_step_hit = true;
        lua_yield(L, 0); // supported: VM lets the hook yield the thread
    }
}

// Re-enables a breakpoint that was temporarily disabled so a resume
// could step past it (the BREAK instruction would otherwise re-fire
// immediately after the hook yields).
void debug_reenable_breakpoint_impl(Runtime* rt, Thread* t) {
    if (!rt->impl_->debug_reenable_line || rt->impl_->debug_reenable_thread != t) return;
    lua_State* co = t->co;
    lua_rawgeti(co, LUA_REGISTRYINDEX, rt->impl_->debug_reenable_ref);
    if (lua_isfunction(co, -1)) lua_breakpoint(co, -1, rt->impl_->debug_reenable_line, 1);
    lua_pop(co, 1);
    lua_unref(co, rt->impl_->debug_reenable_ref);
    rt->impl_->debug_reenable_line = 0;
    rt->impl_->debug_reenable_ref = 0;
    rt->impl_->debug_reenable_thread = nullptr;
}

void debug_reenable_breakpoint(Runtime* rt, Thread* t) {
    debug_reenable_breakpoint_impl(rt, t);
}

std::string debug_format_value(lua_State* L, int idx) {
    std::string error;
    Json v = lua_value_to_json(L, idx, 1, error);
    if (error.empty()) {
        std::string text = v.dump();
        if (text.size() > 160) text = text.substr(0, 157) + "...";
        return text;
    }
    return std::string("<") + lua_typename(L, lua_type(L, idx)) + ">";
}


bool resume_thread(Runtime* rt, Thread* t) {
    lua_State* L = rt->state();
    lua_State* co = t->co;
    int nargs = 0;
    if (t->pending_args >= 0) {
        nargs = t->pending_args;
        t->pending_args = -1;
    } else if (t->wait == Thread::Wait::Time) {
        lua_pushnumber(co, rt->time() - t->slept_at);
        nargs = 1;
    } else if (!t->resume_args.empty()) {
        push_args(co, t->resume_args);
        nargs = static_cast<int>(t->resume_args.size());
    }
    t->resume_args.clear();
    t->wait = Thread::Wait::None;
    t->waiting_on = nullptr;
    rt->impl_->current_owner = t->owner;
    rt->impl_->interrupts = 0;
    int status = lua_resume(co, L, nargs);
    if (status == LUA_YIELD && rt->impl_->debug_break_hit) {
        rt->impl_->debug_break_hit = false;
        rt->impl_->debug_paused_thread = t;
        rt->impl_->debug_pause_reason = "breakpoint";
        rt->impl_->debug_step_mode = 0;
        t->wait = Thread::Wait::Debug;
        return false;
    }
    if (status == LUA_BREAK) {
        // Hit a lua_breakpoint: freeze this thread for the debugger.
        rt->impl_->debug_paused_thread = t;
        rt->impl_->debug_pause_reason = "breakpoint";
        rt->impl_->debug_step_mode = 0;
        t->wait = Thread::Wait::Debug;
        return false;
    }
    if (status == LUA_YIELD && rt->impl_->debug_step_hit &&
        rt->impl_->debug_paused_candidate == t) {
        rt->impl_->debug_step_hit = false;
        rt->impl_->debug_paused_candidate = nullptr;
        lua_singlestep(co, 0);
        rt->impl_->debug_paused_thread = t;
        rt->impl_->debug_pause_reason = "step";
        rt->impl_->debug_step_mode = 0;
        t->wait = Thread::Wait::Debug;
        return false;
    }
    if (status == LUA_OK) {
        debug_reenable_breakpoint(rt, t);
        t->dead = true;
        lua_unref(L, t->co_ref);
        return true;
    }
    if (status == LUA_YIELD) {
        if (t->waiting_on) {
            t->wait = Thread::Wait::Signal;
            return false;
        }
        if (lua_gettop(co) > 0 && lua_isnumber(co, -1)) {
            double delay = lua_tonumber(co, -1);
            lua_pop(co, 1);
            t->wait = Thread::Wait::Time;
            t->slept_at = rt->time();
            t->wake = rt->time() + delay;
            if (delay <= 0) t->ready_step = rt->impl_->step_index + 1;
        } else {
            t->ready_step = rt->impl_->step_index + 1;
        }
        return false;
    }
    std::string msg = lua_tostring(co, -1) ? lua_tostring(co, -1) : "unknown runtime error";
    std::string tb = traceback_of(co, msg);
    rt->report_error(msg, tb, t->owner);
    t->dead = true;
    lua_unref(L, t->co_ref);
    return true;
}

void process_threads(Runtime* rt) {
    auto& threads = rt->impl_->threads;
    for (int guard = 0; guard < 20000; ++guard) {
        bool progress = false;
        for (size_t i = 0; i < threads.size(); ++i) {
            Thread* t = threads[i].get();
            if (t->dead || t->wait == Thread::Wait::Signal || t->wait == Thread::Wait::Debug) continue;
            if (t->wake <= rt->time() && t->ready_step <= rt->impl_->step_index) {
                resume_thread(rt, t);
                progress = true;
            }
        }
        if (!progress) break;
    }
}

void fire_service_signal(Runtime* rt, const char* class_name, const char* key, double dt) {
    for (const auto& child : rt->model().root().children) {
        if (child->class_name == class_name) {
            fire_instance_signal(rt, rt->state(), child.get(), key, {arg_number(dt)});
        }
    }
}


// -------------------------------------------------------------- registration --

void set_method(lua_State* L, const char* table, const char* name, lua_CFunction fn) {
    lua_getfield(L, LUA_REGISTRYINDEX, table);
    lua_pushcfunction(L, fn, name);
    lua_setfield(L, -2, name);
    lua_pop(L, 1);
}

void make_metatable(lua_State* L, const char* name, const char* type_name, lua_CFunction index) {
    luaL_newmetatable(L, name);
    lua_pushstring(L, type_name);
    lua_setfield(L, -2, "__type");
    lua_pushstring(L, type_name);
    lua_setfield(L, -2, "__metatable");
    if (index) {
        lua_pushcfunction(L, index, "__index");
        lua_setfield(L, -2, "__index");
    }
    lua_pop(L, 1);
}

void register_bindings(lua_State* L, Runtime* rt) {
    // Weak instance cache.
    lua_newtable(L);
    lua_newtable(L);
    lua_pushstring(L, "v");
    lua_setfield(L, -2, "__mode");
    lua_setmetatable(L, -2);
    lua_setfield(L, LUA_REGISTRYINDEX, "blox_instcache");
    lua_newtable(L);
    lua_setfield(L, LUA_REGISTRYINDEX, "blox_modulecache");
    lua_newtable(L);
    lua_setfield(L, LUA_REGISTRYINDEX, "blox_enum_items");

    for (const char* table : {"blox_methods", "blox_signal_methods", "blox_connection_methods",
                              "blox_tween_methods", "blox_vector3_methods", "blox_cframe_methods",
                              "blox_track_methods", "blox_datastore_methods"}) {
        lua_newtable(L);
        lua_setfield(L, LUA_REGISTRYINDEX, table);
    }

    // Instance metatable.
    luaL_newmetatable(L, "BloxInstance");
    lua_pushstring(L, "Instance");
    lua_setfield(L, -2, "__type");
    lua_pushstring(L, "Instance");
    lua_setfield(L, -2, "__metatable");
    lua_pushcfunction(L, inst_index, "__index");
    lua_setfield(L, -2, "__index");
    lua_pushcfunction(L, inst_newindex, "__newindex");
    lua_setfield(L, -2, "__newindex");
    lua_pushcfunction(L, inst_eq, "__eq");
    lua_setfield(L, -2, "__eq");
    lua_pushcfunction(L, inst_tostring, "__tostring");
    lua_setfield(L, -2, "__tostring");
    lua_pop(L, 1);

    make_metatable(L, "BloxSignal", "RBXScriptSignal", nullptr);
    luaL_getmetatable(L, "BloxSignal");
    lua_newtable(L);
    lua_pushcfunction(L, sig_connect, "Connect");
    lua_setfield(L, -2, "Connect");
    lua_pushcfunction(L, sig_once, "Once");
    lua_setfield(L, -2, "Once");
    lua_pushcfunction(L, sig_wait, "Wait");
    lua_setfield(L, -2, "Wait");
    lua_setfield(L, -2, "__index"); // methods table as __index (functions take self)
    lua_pop(L, 1);

    make_metatable(L, "BloxConnection", "RBXScriptConnection", conn_index);
    set_method(L, "blox_connection_methods", "Disconnect", conn_disconnect);

    make_metatable(L, "BloxVector3", "Vector3", v3_index);
    luaL_getmetatable(L, "BloxVector3");
    lua_pushcfunction(L, v3_add, "__add"); lua_setfield(L, -2, "__add");
    lua_pushcfunction(L, v3_sub, "__sub"); lua_setfield(L, -2, "__sub");
    lua_pushcfunction(L, v3_mul, "__mul"); lua_setfield(L, -2, "__mul");
    lua_pushcfunction(L, v3_div, "__div"); lua_setfield(L, -2, "__div");
    lua_pushcfunction(L, v3_unm, "__unm"); lua_setfield(L, -2, "__unm");
    lua_pushcfunction(L, v3_eq, "__eq"); lua_setfield(L, -2, "__eq");
    lua_pop(L, 1);
    set_method(L, "blox_vector3_methods", "Lerp", v3_lerp);
    set_method(L, "blox_vector3_methods", "Dot", v3_dot);
    set_method(L, "blox_vector3_methods", "Cross", v3_cross);

    make_metatable(L, "BloxCFrame", "CFrame", cf_index);
    luaL_getmetatable(L, "BloxCFrame");
    lua_pushcfunction(L, cf_mul, "__mul"); lua_setfield(L, -2, "__mul");
    lua_pushcfunction(L, cf_eq, "__eq"); lua_setfield(L, -2, "__eq");
    lua_pop(L, 1);
    set_method(L, "blox_cframe_methods", "components", cf_components);
    set_method(L, "blox_cframe_methods", "GetComponents", cf_components);

    make_metatable(L, "BloxColor3", "Color3", c3_index);
    luaL_getmetatable(L, "BloxColor3");
    lua_pushcfunction(L, c3_eq, "__eq"); lua_setfield(L, -2, "__eq");
    lua_pop(L, 1);
    make_metatable(L, "BloxVector2", "Vector2", v2_index);
    make_metatable(L, "BloxUDim", "UDim", udim_index);
    make_metatable(L, "BloxUDim2", "UDim2", udim2_index);
    make_metatable(L, "BloxEnumItem", "EnumItem", enumitem_index);
    luaL_getmetatable(L, "BloxEnumItem");
    lua_pushcfunction(L, enumitem_eq, "__eq"); lua_setfield(L, -2, "__eq");
    lua_pop(L, 1);
    make_metatable(L, "BloxTween", "Tween", tween_index);
    make_metatable(L, "BloxDataStore", "DataStore", ds_index);
    make_metatable(L, "BloxColorSequence", "ColorSequence", sequence_index);
    make_metatable(L, "BloxNumberSequence", "NumberSequence", sequence_index);
    make_metatable(L, "BloxNumberRange", "NumberRange", nr_index);
    make_metatable(L, "BloxPhysicalProperties", "PhysicalProperties", pp_index);
    // OverlapParams needs __newindex too: build manually below.
    luaL_newmetatable(L, "BloxOverlapParams");
    lua_pushstring(L, "OverlapParams"); lua_setfield(L, -2, "__type");
    lua_pushcfunction(L, op_index, "op_index"); lua_setfield(L, -2, "__index");
    lua_pushcfunction(L, op_newindex, "op_newindex"); lua_setfield(L, -2, "__newindex");
    lua_pop(L, 1);
    set_method(L, "blox_datastore_methods", "GetAsync", ds_get_async);
    set_method(L, "blox_datastore_methods", "SetAsync", ds_set_async);
    set_method(L, "blox_datastore_methods", "UpdateAsync", ds_update_async);
    set_method(L, "blox_datastore_methods", "IncrementAsync", ds_increment_async);
    set_method(L, "blox_datastore_methods", "RemoveAsync", ds_remove_async);
    set_method(L, "blox_datastore_methods", "GetSortedAsync", ds_get_sorted);
    make_metatable(L, "BloxDataStorePages", "DataStorePages", dsp_index);
    set_method(L, "blox_methods", "GetDataStore", m_datastore_get_store);
    set_method(L, "blox_methods", "GetOrderedDataStore", m_datastore_get_ordered);
    set_method(L, "blox_methods", "GetRequestBudgetForRequestType", m_datastore_budget);
    set_method(L, "blox_methods", "JSONEncode", m_http_json_encode);
    set_method(L, "blox_methods", "JSONDecode", m_http_json_decode);
    set_method(L, "blox_methods", "GenerateGUID", m_http_generate_guid);
    set_method(L, "blox_methods", "GetAsync", m_http_request);
    set_method(L, "blox_methods", "PostAsync", m_http_request);
    set_method(L, "blox_methods", "RequestAsync", m_http_request);
    make_metatable(L, "BloxRegion3", "Region3", region3_index);
    luaL_newmetatable(L, "BloxRaycastParams");
    lua_pushstring(L, "RaycastParams"); lua_setfield(L, -2, "__type");
    lua_pushstring(L, "RaycastParams"); lua_setfield(L, -2, "__metatable");
    lua_pushcfunction(L, rcp_index, "__index"); lua_setfield(L, -2, "__index");
    lua_pushcfunction(L, rcp_newindex, "__newindex"); lua_setfield(L, -2, "__newindex");
    lua_pop(L, 1);
    luaL_newmetatable(L, "BloxAnimationTrack");
    lua_pushstring(L, "AnimationTrack"); lua_setfield(L, -2, "__type");
    lua_pushstring(L, "AnimationTrack"); lua_setfield(L, -2, "__metatable");
    lua_pushcfunction(L, track_index, "__index"); lua_setfield(L, -2, "__index");
    lua_pushcfunction(L, track_newindex, "__newindex"); lua_setfield(L, -2, "__newindex");
    lua_pop(L, 1);
    set_method(L, "blox_track_methods", "Play", track_play);
    set_method(L, "blox_track_methods", "Stop", track_stop);
    set_method(L, "blox_track_methods", "AdjustSpeed", track_adjust_speed);
    set_method(L, "blox_track_methods", "AdjustWeight", track_adjust_weight);
    luaL_newmetatable(L, "BloxPath");
    lua_pushstring(L, "Path"); lua_setfield(L, -2, "__type");
    lua_pushstring(L, "Path"); lua_setfield(L, -2, "__metatable");
    lua_pushcfunction(L, path_index, "__index"); lua_setfield(L, -2, "__index");
    lua_pop(L, 1);
    lua_newtable(L); lua_setfield(L, LUA_REGISTRYINDEX, "blox_path_methods");
    set_method(L, "blox_path_methods", "ComputeAsync", m_path_compute);
    set_method(L, "blox_path_methods", "GetWaypoints", m_path_get_waypoints);
    set_method(L, "blox_tween_methods", "Play", tween_play);
    set_method(L, "blox_tween_methods", "Pause", tween_pause);
    set_method(L, "blox_tween_methods", "Cancel", tween_cancel);

    // Instance methods.
    set_method(L, "blox_methods", "FindFirstChild", m_find_first_child);
    set_method(L, "blox_methods", "FindFirstChildOfClass", m_find_first_child_of_class);
    set_method(L, "blox_methods", "GetChildren", m_get_children);
    set_method(L, "blox_methods", "GetDescendants", m_get_descendants);
    set_method(L, "blox_methods", "IsA", m_is_a);
    set_method(L, "blox_methods", "Clone", m_clone);
    set_method(L, "blox_methods", "Destroy", m_destroy);
    set_method(L, "blox_methods", "ClearAllChildren", m_clear_children);
    set_method(L, "blox_methods", "GetFullName", m_get_full_name);
    set_method(L, "blox_methods", "GetService", m_get_service);
    set_method(L, "blox_methods", "GetAttribute", m_get_attribute);
    set_method(L, "blox_methods", "SetAttribute", m_set_attribute);
    set_method(L, "blox_methods", "GetPropertyChangedSignal", m_get_property_changed_signal);
    set_method(L, "blox_methods", "AddTag", m_add_tag);
    set_method(L, "blox_methods", "RemoveTag", m_remove_tag);
    set_method(L, "blox_methods", "HasTag", m_has_tag);
    set_method(L, "blox_methods", "GetTags", m_get_tags);
    // Service-specific methods (harmless on other classes; they delegate).
    set_method(L, "blox_methods", "Create", m_tween_create);
    set_method(L, "blox_methods", "GetTagged", m_collection_get_tagged);
    set_method(L, "blox_methods", "GetPlayers", m_players_get_players);
    set_method(L, "blox_methods", "IsRunning", m_runservice_is_running);
    set_method(L, "blox_methods", "IsServer", m_runservice_is_server);
    set_method(L, "blox_methods", "IsClient", m_runservice_is_client);
    set_method(L, "blox_methods", "IsKeyDown", m_uis_is_key_down);
    set_method(L, "blox_methods", "BindToRenderStep", m_unsupported);
    // CollectionService tag methods shadow the instance ones by signature;
    // instance AddTag handles (inst, tag), service handles (svc, inst, tag):
    // both registered under the same names is ambiguous, so the service
    // variants live under distinct internal entries wired below.
    lua_getfield(L, LUA_REGISTRYINDEX, "blox_methods");
    lua_pushstring(L, "added:");
    lua_pushcclosure(L, m_collection_tag_signal, "GetInstanceAddedSignal", 1);
    lua_setfield(L, -2, "GetInstanceAddedSignal");
    lua_pushstring(L, "removed:");
    lua_pushcclosure(L, m_collection_tag_signal, "GetInstanceRemovedSignal", 1);
    lua_setfield(L, -2, "GetInstanceRemovedSignal");
    lua_pop(L, 1);

    // Enum registry.
    static const std::map<std::string, std::vector<std::pair<std::string, int64_t>>> kEnums = {
        {"Material", {{"Plastic", 256}, {"SmoothPlastic", 272}, {"Neon", 288}, {"Wood", 512},
                      {"WoodPlanks", 528}, {"Marble", 784}, {"Slate", 800}, {"CrackedLava", 804},
                      {"Concrete", 816}, {"Granite", 832}, {"Brick", 848}, {"CorrodedMetal", 1040},
                      {"DiamondPlate", 1056}, {"Foil", 1072}, {"Metal", 1088}, {"Grass", 1280},
                      {"LeafyGrass", 1284}, {"Fabric", 1296}, {"Sand", 1312}, {"Snow", 1328},
                      {"Ice", 1536}, {"Glacier", 1540}, {"Glass", 1568}, {"ForceField", 1584},
                      {"Air", 1792}, {"Water", 2048}}},
        {"PartType", {{"Ball", 0}, {"Block", 1}, {"Cylinder", 2}}},
        {"RaycastFilterType", {{"Exclude", 0}, {"Include", 1}}},
        {"PathStatus", {{"Success", 0}, {"ClosestStart", 1}, {"ClosestGoal", 2}, {"NoPath", 3}}},
        {"PathWaypointAction", {{"Walk", 0}, {"Jump", 1}}},
        {"UserInputState", {{"Begin", 0}, {"Change", 1}, {"End", 2}, {"Cancel", 3}}},
        {"UserInputType", {{"MouseButton1", 0}, {"MouseButton2", 1}, {"MouseButton3", 2},
                           {"MouseWheel", 3}, {"MouseMovement", 4}, {"Touch", 7}, {"Keyboard", 8},
                           {"Gamepad1", 12}, {"None", 18}}},
        {"HumanoidStateType", {{"FallingDown", 0}, {"Running", 1}, {"RunningNoPhysics", 2},
                               {"Climbing", 3}, {"StrafingNoPhysics", 4}, {"Ragdoll", 5},
                               {"GettingUp", 6}, {"Jumping", 7}, {"Landed", 8}, {"Flying", 9},
                               {"Freefall", 10}, {"Seated", 11}, {"PlatformStanding", 12},
                               {"Dead", 13}, {"Swimming", 14}, {"Physics", 15}, {"None", 16}}},
        {"Font", {{"Legacy", 0}, {"Arial", 1}, {"ArialBold", 2}, {"SourceSans", 3},
                  {"SourceSansBold", 4}, {"Gotham", 33}, {"GothamBold", 35}, {"Code", 15}}},
        {"HumanoidRigType", {{"R6", 0}, {"R15", 1}}},
        {"EasingStyle", {{"Linear", 0}, {"Sine", 1}, {"Back", 2}, {"Quad", 3}, {"Quart", 4},
                         {"Quint", 5}, {"Bounce", 6}, {"Elastic", 7}, {"Exponential", 8},
                         {"Circular", 9}, {"Cubic", 10}}},
        {"EasingDirection", {{"In", 0}, {"Out", 1}, {"InOut", 2}}},
        {"DataStoreRequestType", {{"GetAsync", 0}, {"SetIncrementAsync", 1}, {"UpdateAsync", 2},
                                  {"GetSortedAsync", 3}, {"SetIncrementSortedAsync", 4}, {"OnUpdate", 5}}},
        {"PlaybackState", {{"Begin", 0}, {"Delayed", 1}, {"Playing", 2}, {"Paused", 3},
                           {"Completed", 4}, {"Cancelled", 5}}},
        {"KeyCode", {{"W", 87}, {"A", 65}, {"S", 83}, {"D", 68}, {"Space", 32}, {"Up", 273},
                     {"Down", 274}, {"Left", 276}, {"Right", 275}, {"LeftShift", 304},
                     {"E", 69}, {"Q", 81}, {"F", 70}, {"R", 82}}},
    };
    lua_newtable(L); // Enum
    for (const auto& [type_name, items] : kEnums) {
        lua_newtable(L);
        for (const auto& [item_name, value] : items) {
            push_enum_item(L, type_name, item_name, value);
            lua_pushvalue(L, -1);
            lua_setfield(L, -3, item_name.c_str());
            // registry map "Type:value" -> item (shared userdata)
            lua_getfield(L, LUA_REGISTRYINDEX, "blox_enum_items");
            lua_pushstring(L, (type_name + ":" + std::to_string(value)).c_str());
            lua_pushvalue(L, -3);
            lua_rawset(L, -3);
            lua_pop(L, 2); // enum-items table + the item itself
        }
        lua_setfield(L, -2, type_name.c_str());
    }
    lua_setglobal(L, "Enum");

    // Datatype constructor tables.
    lua_newtable(L);
    lua_pushcfunction(L, v3_new, "new"); lua_setfield(L, -2, "new");
    push_vector3(L, 0, 0, 0); lua_setfield(L, -2, "zero");
    push_vector3(L, 1, 1, 1); lua_setfield(L, -2, "one");
    lua_setglobal(L, "Vector3");

    lua_newtable(L);
    lua_pushcfunction(L, cf_new, "new"); lua_setfield(L, -2, "new");
    lua_pushcfunction(L, cf_look_at, "lookAt"); lua_setfield(L, -2, "lookAt");
    lua_pushcfunction(L, cf_angles, "Angles"); lua_setfield(L, -2, "Angles");
    lua_pushcfunction(L, cf_angles, "fromEulerAnglesXYZ"); lua_setfield(L, -2, "fromEulerAnglesXYZ");
    lua_setglobal(L, "CFrame");

    lua_newtable(L);
    lua_pushcfunction(L, c3_new, "new"); lua_setfield(L, -2, "new");
    lua_pushcfunction(L, c3_from_rgb, "fromRGB"); lua_setfield(L, -2, "fromRGB");
    lua_setglobal(L, "Color3");

    lua_newtable(L);
    lua_pushcfunction(L, v2_new, "new"); lua_setfield(L, -2, "new");
    push_vector2(L, 0, 0); lua_setfield(L, -2, "zero");
    push_vector2(L, 1, 1); lua_setfield(L, -2, "one");
    lua_setglobal(L, "Vector2");

    lua_newtable(L);
    lua_pushcfunction(L, udim_new, "new"); lua_setfield(L, -2, "new");
    lua_setglobal(L, "UDim");

    lua_newtable(L);
    lua_pushcfunction(L, udim2_new, "new"); lua_setfield(L, -2, "new");
    lua_pushcfunction(L, udim2_from_scale, "fromScale"); lua_setfield(L, -2, "fromScale");
    lua_pushcfunction(L, udim2_from_offset, "fromOffset"); lua_setfield(L, -2, "fromOffset");
    lua_setglobal(L, "UDim2");

    lua_newtable(L);
    lua_pushcfunction(L, fn_instance_new, "new"); lua_setfield(L, -2, "new");
    lua_setglobal(L, "Instance");
    lua_newtable(L);
    lua_pushcfunction(L, m_raycast_params_new, "new"); lua_setfield(L, -2, "new");
    lua_setglobal(L, "RaycastParams");

    lua_newtable(L);
    lua_pushcfunction(L, region3_new, "new"); lua_setfield(L, -2, "new");
    lua_setglobal(L, "Region3");

    set_method(L, "blox_methods", "LoadAnimation", m_animator_load_animation);
    set_method(L, "blox_methods", "GetState", m_humanoid_get_state);
    set_method(L, "blox_methods", "CreatePath", m_pathfinding_create_path);
    set_method(L, "blox_methods", "BindAction", m_cas_bind_action);
    set_method(L, "blox_methods", "UnbindAction", m_cas_unbind_action);
    set_method(L, "blox_methods", "UnbindAllActions", m_cas_unbind_all);
    set_method(L, "blox_methods", "RegisterCollisionGroup", m_physics_register_group);
    set_method(L, "blox_methods", "SetPartCollisionGroup", m_physics_set_part_group);
    set_method(L, "blox_methods", "CollisionGroupSetCollidable", m_physics_set_collidable);
    set_method(L, "blox_methods", "CollisionGroupsAreCollidable", m_physics_are_collidable);
    set_method(L, "blox_methods", "ChangeState", m_humanoid_change_state);
    set_method(L, "blox_methods", "FillBlock", m_terrain_fill_block);
    set_method(L, "blox_methods", "FillBall", m_terrain_fill_ball);
    set_method(L, "blox_methods", "Clear", m_terrain_clear);
    set_method(L, "blox_methods", "ReadVoxels", m_terrain_read_voxels);
    set_method(L, "blox_methods", "FillCylinder", m_terrain_fill_cylinder);
    set_method(L, "blox_methods", "FillWedge", m_terrain_fill_wedge);
    set_method(L, "blox_methods", "FillRegion", m_terrain_fill_region);
    set_method(L, "blox_methods", "WriteVoxels", m_terrain_write_voxels);
    set_method(L, "blox_methods", "CopyRegion", m_terrain_copy_region);
    set_method(L, "blox_methods", "PasteRegion", m_terrain_paste_region);
    set_method(L, "blox_methods", "GetTextSize", m_text_service_get_text_size);
    set_method(L, "blox_methods", "Raycast", m_workspace_raycast);
    set_method(L, "blox_methods", "GetPartBoundsInBox", m_workspace_get_parts_in_box);
    set_method(L, "blox_methods", "GetPartBoundsInRadius", m_workspace_get_parts_in_radius);
    set_method(L, "blox_methods", "FireServer", m_remote_fire_server);
    set_method(L, "blox_methods", "FireClient", m_remote_fire_client);
    set_method(L, "blox_methods", "FireAllClients", m_remote_fire_all_clients);
    set_method(L, "blox_methods", "InvokeServer", m_remote_invoke_server);
    set_method(L, "blox_methods", "InvokeClient", m_remote_invoke_client);

    lua_newtable(L);
    lua_pushcfunction(L, fn_tween_info_new, "new"); lua_setfield(L, -2, "new");
    lua_setglobal(L, "TweenInfo");

    lua_newtable(L);
    lua_pushcfunction(L, cs_new, "new"); lua_setfield(L, -2, "new");
    lua_setglobal(L, "ColorSequence");
    lua_newtable(L);
    lua_pushcfunction(L, cs_keypoint_new, "new"); lua_setfield(L, -2, "new");
    lua_setglobal(L, "ColorSequenceKeypoint");
    lua_newtable(L);
    lua_pushcfunction(L, ns_new, "new"); lua_setfield(L, -2, "new");
    lua_setglobal(L, "NumberSequence");
    lua_newtable(L);
    lua_pushcfunction(L, ns_keypoint_new, "new"); lua_setfield(L, -2, "new");
    lua_setglobal(L, "NumberSequenceKeypoint");
    lua_newtable(L);
    lua_pushcfunction(L, nr_new, "new"); lua_setfield(L, -2, "new");
    lua_setglobal(L, "NumberRange");
    lua_newtable(L);
    lua_pushcfunction(L, pp_new, "new"); lua_setfield(L, -2, "new");
    lua_setglobal(L, "PhysicalProperties");
    lua_newtable(L);
    lua_pushcfunction(L, op_new, "new"); lua_setfield(L, -2, "new");
    lua_setglobal(L, "OverlapParams");

    lua_newtable(L);
    lua_pushcfunction(L, task_wait, "wait"); lua_setfield(L, -2, "wait");
    lua_pushcfunction(L, task_spawn, "spawn"); lua_setfield(L, -2, "spawn");
    lua_pushcfunction(L, task_defer, "defer"); lua_setfield(L, -2, "defer");
    lua_pushcfunction(L, task_delay, "delay"); lua_setfield(L, -2, "delay");
    lua_pushcfunction(L, task_clock, "clock"); lua_setfield(L, -2, "clock");
    lua_setglobal(L, "task");

    lua_pushcfunction(L, fn_print, "print"); lua_setglobal(L, "print");
    lua_pushcfunction(L, fn_warn, "warn"); lua_setglobal(L, "warn");
    lua_pushcfunction(L, fn_tick, "tick"); lua_setglobal(L, "tick");
    lua_pushcfunction(L, fn_typeof, "typeof"); lua_setglobal(L, "typeof");
    lua_pushcfunction(L, fn_require, "require"); lua_setglobal(L, "require");

    // Sandbox surface: no filesystem/process/dynamic code loading.
    lua_pushnil(L); lua_setglobal(L, "io");
    lua_pushnil(L); lua_setglobal(L, "loadstring");
    lua_pushnil(L); lua_setglobal(L, "load");
    lua_newtable(L);
    lua_pushcfunction(L, fn_tick, "clock"); lua_setfield(L, -2, "clock");
    lua_setglobal(L, "os");

    push_instance(L, &rt->model().root());
    lua_setglobal(L, "game");
    for (const auto& child : rt->model().root().children) {
        if (child->class_name == "Workspace") {
            push_instance(L, child.get());
            lua_setglobal(L, "workspace");
            break;
        }
    }
}

// Compiles `source` onto thread `co`. Returns false with an error message.
bool load_chunk(lua_State* co, const std::string& source, const std::string& chunk_name,
                std::string& error) {
    lua_CompileOptions opts{};
    opts.optimizationLevel = 1;
    opts.debugLevel = 2; // tracebacks with line info
    size_t bc_size = 0;
    char* bc = luau_compile(source.data(), source.size(), &opts, &bc_size);
    int status = luau_load(co, chunk_name.c_str(), bc, bc_size, 0);
    std::free(bc);
    if (status != 0) {
        error = lua_tostring(co, -1) ? lua_tostring(co, -1) : "compile error";
        lua_pop(co, 1);
        return false;
    }
    return true;
}

const char* kBootstrap = R"LUau(
local methods = ...
methods.WaitForChild = function(self, name, timeout)
    local waited = 0
    while true do
        local child = self:FindFirstChild(name)
        if child then return child end
        if timeout ~= nil and waited >= timeout then
            error("WaitForChild: '" .. tostring(name) .. "' was not found before timeout", 2)
        end
        waited += task.wait(0.03)
    end
end
)LUau";


// ---------------------------------------------------------------- Runtime --

Runtime::Runtime(DataModel& model) : model_(model), impl_(std::make_unique<Impl>()) {
    L_ = luaL_newstate();
    luaL_openlibs(L_);
    lua_pushlightuserdata(L_, this);
    lua_setfield(L_, LUA_REGISTRYINDEX, "blox_rt");
    lua_callbacks(L_)->interrupt = [](lua_State* L, int gc) {
        (void)gc;
        Runtime* rt = rt_of(L);
        if (!rt) return;
        Impl& impl = *rt->impl_;
        if (++impl.interrupts > impl.interrupt_budget) {
            impl.interrupts = 0;
            luaL_error(L, "script execution limit exceeded (possible infinite loop)");
        }
        if ((impl.interrupts & 0x3FFF) == 0 && lua_totalbytes(L, 0) > (512ull << 20)) {
            luaL_error(L, "script memory limit exceeded (512 MiB)");
        }
    };
    register_bindings(L_, this);
    // Bootstrap Luau-side method wrappers (WaitForChild).
    {
        lua_getfield(L_, LUA_REGISTRYINDEX, "blox_methods");
        std::string error;
        lua_State* co = lua_newthread(L_);
        if (load_chunk(co, kBootstrap, "=blox_bootstrap", error)) {
            lua_pushvalue(L_, -2); // methods table
            lua_xmove(L_, co, 1);
            if (lua_pcall(co, 1, 0, 0) != LUA_OK) {
                log("warn", std::string("bootstrap failed: ") + (lua_tostring(co, -1) ? lua_tostring(co, -1) : "?"), "");
            }
        }
        lua_pop(L_, 2); // thread + methods table
    }
    luaL_sandbox(L_);
}

Runtime::~Runtime() {
    if (L_) lua_close(L_);
}



void Runtime::set_humanoid_state(Instance* humanoid, const std::string& state) {
    if (!humanoid) return;
    std::string& current = impl_->humanoid_states[humanoid];
    if (current == state) return;
    std::string old = current.empty() ? "Running" : current;
    current = state;
    fire_instance_signal(this, L_, humanoid, "StateChanged",
                         {enum_arg("HumanoidStateType", state), enum_arg("HumanoidStateType", old)});
    process_threads(this);
}

bool Runtime::animation_playing_in(Instance* model_root) {
    if (!model_root) return false;
    for (auto& track : impl_->anim_tracks) {
        if (track->stopped) continue;
        for (Instance* cur = track->animator; cur; cur = cur->parent)
            if (cur == model_root) return true;
    }
    return false;
}

Runtime* Runtime::from_state(lua_State* L) { return rt_of(L); }

void Runtime::log(std::string level, std::string message, std::string source) {
    if (logs_.size() > 10000) logs_.erase(logs_.begin());
    logs_.push_back({std::move(level), std::move(message), std::move(source)});
}

void Runtime::report_error(std::string message, std::string traceback, std::string source) {
    errors_.push_back({std::move(message), std::move(traceback), std::move(source)});
}

bool Runtime::any_thread_alive() const {
    for (const auto& t : impl_->threads) {
        if (!t->dead) return true;
    }
    return false;
}

size_t Runtime::thread_count() const {
    size_t n = 0;
    for (const auto& t : impl_->threads) {
        if (!t->dead) ++n;
    }
    return n;
}


// ------------------------------------------------------------- networking --

void Runtime::set_net_context(NetContext context, int player_index) {
    impl_->net_context = context;
    impl_->player_index = player_index;
}
Runtime::NetContext Runtime::net_context() const { return impl_->net_context; }
int Runtime::player_index() const { return impl_->player_index; }
void Runtime::set_local_player(Instance* player) { impl_->local_player = player; }
void Runtime::set_remote_sink(std::function<void(const RemoteCall&)> sink) {
    impl_->remote_sink = std::move(sink);
}
void Runtime::set_character_resolver(std::function<Instance*(Instance*)> resolver) {
    impl_->character_resolver = std::move(resolver);
}
void Runtime::set_physics(PhysicsWorld* physics) { impl_->physics = physics; }
void Runtime::set_datastore_dir(const std::string& dir) { impl_->datastore_dir = dir; }
const std::string& Runtime::datastore_dir() const { return impl_->datastore_dir; }

std::vector<LuaArg> json_args_to_lua_args(const std::string& args_json) {
    std::vector<LuaArg> out;
    Json parsed;
    std::string error;
    if (!Json::parse(args_json, parsed, error) || !parsed.is_array()) return out;
    for (const Json& v : parsed.as_array()) {
        if (v.is_null()) out.push_back(LuaArg{});
        else if (v.is_bool()) { LuaArg a; a.kind = 1; a.b = v.as_bool(); out.push_back(a); }
        else if (v.is_number()) { LuaArg a; a.kind = 2; a.num = v.as_double(); out.push_back(a); }
        else if (v.is_string()) { LuaArg a; a.kind = 3; a.str = v.as_string(); out.push_back(a); }
        else out.push_back(arg_json(v.dump()));
    }
    return out;
}

void Runtime::deliver_remote(Instance* remote, Instance* from_player, const std::string& args_json) {
    if (!remote) return;
    auto args = json_args_to_lua_args(args_json);
    if (impl_->net_context != NetContext::Client) {
        // Server side: handlers receive (player, ...args).
        LuaArg player_arg;
        if (from_player) { player_arg.kind = 4; player_arg.inst = from_player; }
        args.insert(args.begin(), player_arg);
        fire_instance_signal(this, L_, remote, "OnServerEvent", args);
    } else {
        fire_instance_signal(this, L_, remote, "OnClientEvent", args);
    }
    process_threads(this);
}

void Runtime::fire_signal_json(Instance* inst, const std::string& key, const std::string& args_json) {
    if (!inst) return;
    fire_instance_signal(this, L_, inst, key, json_args_to_lua_args(args_json));
    process_threads(this);
}

std::string Runtime::invoke_remote_handler(Instance* remote, Instance* from_player,
                                           const std::string& args_json, bool& ok) {
    ok = false;
    auto it = impl_->remote_invoke_handlers.find(remote);
    if (it == impl_->remote_invoke_handlers.end()) {
        report_error("RemoteFunction '" + remote->name + "' has no invoke handler assigned",
                     "", model_.path_of(*remote));
        return "";
    }
    Thread* t = make_thread(this, L_, model_.path_of(*remote));
    lua_State* co = t->co;
    lua_rawgeti(co, LUA_REGISTRYINDEX, it->second);
    auto args = json_args_to_lua_args(args_json);
    if (impl_->net_context != NetContext::Client && from_player) {
        LuaArg player_arg;
        player_arg.kind = 4;
        player_arg.inst = from_player;
        args.insert(args.begin(), player_arg);
    }
    push_args(co, args);
    t->pending_args = static_cast<int>(args.size());
    resume_thread(this, t);
    int guard = 0;
    while (!t->dead && guard++ < 1000) {
        if (t->wait == Thread::Wait::Signal) break;
        t->wake = time_;
        t->ready_step = 0;
        resume_thread(this, t);
    }
    if (!t->dead) return "";
    if (!errors_.empty() && errors_.back().source == model_.path_of(*remote)) return "";
    // Collect return values from the finished thread stack.
    std::string error;
    Json out = Json::array();
    int top = lua_gettop(co);
    for (int i = 1; i <= top; ++i) {
        Json v = lua_value_to_json(co, i, 0, error);
        if (!error.empty()) return "";
        out.push(std::move(v));
    }
    ok = true;
    return out.dump();
}

void Runtime::complete_invoke(int invoke_id, const std::string& result_json, bool ok,
                              const std::string& error) {
    auto it = impl_->pending_invokes.find(invoke_id);
    if (it == impl_->pending_invokes.end()) return;
    Thread* t = it->second;
    impl_->pending_invokes.erase(it);
    if (t->dead) return;
    if (!ok) {
        // A failed invoke kills the calling thread (documented): the
        // error is reported with the caller's script as the source.
        report_error("RemoteFunction invoke failed: " + error, "", t->owner);
        t->dead = true;
        lua_unref(L_, t->co_ref);
        return;
    }
    t->wait = Thread::Wait::None;
    t->waiting_on = nullptr;
    t->resume_args = json_args_to_lua_args(result_json);
    t->wake = time_;
    t->ready_step = 0;
}

bool Runtime::start_scripts() {
    struct Found {
        Instance* inst;
        bool local;
    };
    std::vector<Found> server, client;
    auto under_starter = [](const Instance* inst) {
        for (const Instance* cur = inst->parent; cur; cur = cur->parent) {
            if (cur->class_name == "StarterGui" || cur->class_name == "StarterPlayer" ||
                cur->class_name == "StarterPack" || cur->class_name == "ReplicatedFirst")
                return true;
        }
        return false;
    };
    std::vector<Instance*> stack{&model_.root()};
    std::vector<Instance*> ordered;
    while (!stack.empty()) {
        Instance* cur = stack.back();
        stack.pop_back();
        ordered.push_back(cur);
        for (auto it = cur->children.rbegin(); it != cur->children.rend(); ++it) stack.push_back(it->get());
    }
    for (Instance* inst : ordered) {
        if (inst->class_name == "Script") {
            if (!under_starter(inst)) server.push_back({inst, false});
        } else if (inst->class_name == "LocalScript") {
            if (under_starter(inst)) client.push_back({inst, true});
            else log("warn", "LocalScript '" + model_.path_of(*inst) +
                                 "' is not in a client container (StarterGui/StarterPlayer/...); not run",
                     model_.path_of(*inst));
        }
    }
    auto start_one = [&](Instance* inst) {
        auto source_it = inst->properties.find("Source");
        std::string source = (source_it != inst->properties.end() &&
                              source_it->second.type == ValueType::String)
                                 ? source_it->second.string_value
                                 : "";
        if (source.empty()) return;
        std::string path = model_.path_of(*inst);
        Thread* t = make_thread(this, L_, path);
        push_instance(t->co, inst);
        lua_setglobal(t->co, "script");
        std::string error;
        if (!load_chunk(t->co, source, "=" + path, error)) {
            report_error(error, error, path);
            t->dead = true;
            return;
        }
        if (impl_->debug_mode) {
            // Breakpoints apply to the script's main chunk function
            // (the closure luau_load just left on the stack). Nested
            // function bodies are reached with step-into.
            auto it = impl_->debug_breakpoints.find(path);
            if (it != impl_->debug_breakpoints.end()) {
                for (int line : it->second) lua_breakpoint(t->co, -1, line, 1);
            }
        }
        resume_thread(this, t);
    };
    if (impl_->net_context == NetContext::Client) {
        // Client contexts run LocalScripts (and nothing else) — server
        // Scripts in the replicated tree stay inert here, exactly like a
        // real client that received them but never executes them.
        for (const Found& f : client) start_one(f.inst);
    } else if (impl_->net_context == NetContext::Server) {
        for (const Found& f : server) start_one(f.inst);
    } else {
        for (const Found& f : server) start_one(f.inst);
        for (const Found& f : client) start_one(f.inst);
    }
    return true;
}

void Runtime::debug_set_breakpoints(const std::map<std::string, std::vector<int>>& breakpoints) {
    impl_->debug_breakpoints.clear();
    for (const auto& [script, lines] : breakpoints) {
        impl_->debug_breakpoints[script] = std::set<int>(lines.begin(), lines.end());
    }
    impl_->debug_mode = true;
    lua_Callbacks* cb = lua_callbacks(L_);
    cb->debugbreak = &debug_break_hook;
    cb->debugstep = &debug_step_hook;
    g_debug_runtime = this;
}

Runtime::DebugStatus Runtime::debug_status(int frame, std::vector<DebugVariable>* variables) const {
    DebugStatus out;
    Thread* paused = impl_->debug_paused_thread;
    out.paused = paused != nullptr;
    out.reason = impl_->debug_pause_reason;
    bool any_alive = false;
    for (const auto& t : impl_->threads) {
        if (!t->dead) any_alive = true;
    }
    out.finished = !any_alive && !out.paused;
    if (!paused) return out;
    lua_State* co = paused->co;
    for (int level = 0; level < 64; ++level) {
        lua_Debug info;
        if (!lua_getinfo(co, level, "nsl", &info)) break;
        DebugFrame f;
        f.function = info.name ? info.name : "?";
        f.source = info.source ? info.source : "";
        if (!f.source.empty() && f.source[0] == '=') f.source = f.source.substr(1);
        f.line = info.currentline;
        out.stack.push_back(std::move(f));
    }
    if (!out.stack.empty()) {
        out.script = out.stack[0].source;
        out.line = out.stack[0].line;
    }
    if (impl_->debug_pause_line > 0) {
        // Location captured inside the hook at the moment of the pause
        // (the post-yield stack walk reads the previous instruction).
        out.line = impl_->debug_pause_line;
        std::string script = impl_->debug_pause_script;
        if (!script.empty() && script[0] == '=') script = script.substr(1);
        if (!script.empty()) out.script = script;
        if (!out.stack.empty()) out.stack[0].line = impl_->debug_pause_line;
    }
    if (variables) {
        // Locals of the requested frame.
        for (int i = 1; i < 256; ++i) {
            const char* name = lua_getlocal(co, frame, i);
            if (!name) break;
            DebugVariable var;
            var.name = name;
            var.kind = "local";
            var.value = debug_format_value(co, -1);
            lua_pop(co, 1);
            variables->push_back(std::move(var));
        }
        // Upvalues of the frame's function.
        lua_Debug info;
        if (lua_getinfo(co, frame, "f", &info)) {
            for (int i = 1; i < 256; ++i) {
                const char* name = lua_getupvalue(co, -1, i);
                if (!name) break;
                DebugVariable var;
                var.name = name;
                var.kind = "upvalue";
                var.value = debug_format_value(co, -1);
                lua_pop(co, 1);
                variables->push_back(std::move(var));
            }
            lua_pop(co, 1); // the function
        }
    }
    return out;
}

bool Runtime::debug_resume(const std::string& mode) {
    Thread* t = impl_->debug_paused_thread;
    if (!t) return false;
    const bool from_breakpoint = impl_->debug_pause_reason == "breakpoint";
    const int pause_line = impl_->debug_pause_line;
    if (from_breakpoint && pause_line > 0) {
        // Temporarily disable the breakpoint we are sitting on (the
        // BREAK instruction re-fires on resume); the step hook
        // re-enables it once execution leaves this line.
        lua_Debug info;
        if (lua_getinfo(t->co, 0, "f", &info) && lua_isfunction(t->co, -1)) {
            lua_breakpoint(t->co, -1, pause_line, 0);
            impl_->debug_reenable_ref = lua_ref(t->co, -1);
            impl_->debug_reenable_line = pause_line;
            impl_->debug_reenable_thread = t;
        }
        lua_pop(t->co, 1);
    }
    impl_->debug_paused_thread = nullptr;
    impl_->debug_pause_reason.clear();
    impl_->debug_pause_line = 0;
    impl_->debug_pause_script.clear();
    t->wait = Thread::Wait::None;
    if (mode == "continue" && from_breakpoint) {
        // Skip-once: single-step until execution leaves the breakpoint
        // line, so the same BREAK does not immediately re-pause.
        impl_->debug_step_mode = 4;
        impl_->debug_step_depth = lua_stackdepth(t->co);
        impl_->debug_step_line = pause_line;
        impl_->debug_step_hit = false;
        impl_->debug_paused_candidate = t;
        lua_singlestep(t->co, 1);
    } else if (mode == "continue") {
        impl_->debug_step_mode = 0;
        lua_singlestep(t->co, 0);
    } else {
        impl_->debug_step_mode = mode == "stepInto" ? 1 : mode == "stepOver" ? 2 : 3;
        impl_->debug_step_depth = lua_stackdepth(t->co);
        // Compare against the line captured inside the hook at pause
        // time; a post-yield stack walk reads the previous instruction.
        if (pause_line > 0) {
            impl_->debug_step_line = pause_line;
        } else {
            lua_Debug info;
            impl_->debug_step_line = lua_getinfo(t->co, 0, "l", &info) ? info.currentline : -1;
        }
        impl_->debug_step_hit = false;
        impl_->debug_paused_candidate = t;
        lua_singlestep(t->co, 1);
    }
    resume_thread(this, t);
    process_threads(this);
    if (impl_->debug_step_mode != 0 && !impl_->debug_paused_thread) {
        // Stepping ended without another pause (thread finished or
        // yielded for time/signal): stop single-stepping.
        impl_->debug_step_mode = 0;
        impl_->debug_paused_candidate = nullptr;
        lua_singlestep(t->co, 0);
    }
    return true;
}

bool Runtime::eval(const std::string& source, const std::string& chunk_name, std::string& error,
                   std::string& result_json) {
    Thread* t = make_thread(this, L_, chunk_name);
    std::string load_error;
    if (!load_chunk(t->co, source, "=" + chunk_name, load_error)) {
        error = load_error;
        report_error(load_error, load_error, chunk_name);
        t->dead = true;
        return false;
    }
    resume_thread(this, t);
    // Fast-forward: keep resuming while the chunk yields on waits, so
    // eval behaves like running code to completion.
    int guard = 0;
    while (!t->dead && guard++ < 1000) {
        if (t->wait == Thread::Wait::Signal) break;
        t->wake = time_;
        t->ready_step = 0;
        resume_thread(this, t);
    }
    if (!t->dead) {
        error = "eval chunk is still waiting (event or long task); use `run` with steps instead";
        return false;
    }
    if (!errors_.empty() && errors_.back().source == chunk_name) {
        error = errors_.back().message;
        return false;
    }
    // Serialize leftover stack values (return values) compactly.
    Json arr = Json::array();
    int top = lua_gettop(t->co);
    for (int i = 1; i <= top; ++i) {
        if (lua_isstring(t->co, i)) arr.push(Json(lua_tostring(t->co, i)));
        else if (lua_isnumber(t->co, i)) arr.push(Json(lua_tonumber(t->co, i)));
        else if (lua_isboolean(t->co, i)) arr.push(Json(lua_toboolean(t->co, i) != 0));
        else if (lua_isnil(t->co, i)) arr.push(Json());
        else arr.push(Json(std::string("<") + luaL_typename(t->co, i) + ">"));
    }
    result_json = arr.dump();
    return true;
}

void Runtime::fire_event(Instance* inst, const std::string& signal_key) {
    fire_instance_signal(this, L_, inst, signal_key, {});
}

void Runtime::notify_moved(const std::vector<Instance*>& moved) {
    for (Instance* inst : moved) {
        fire_changed(this, L_, inst, "CFrame");
    }
}

void Runtime::set_key_state(const std::string& key_name, bool down) {
    bool was = impl_->key_states[key_name];
    impl_->key_states[key_name] = down;
    if (was == down) return;
    const char* signal = down ? "InputBegan" : "InputEnded";
    for (const auto& child : model_.root().children) {
        if (child->class_name == "UserInputService") {
            fire_instance_signal(this, L_, child.get(), signal, {arg_string(key_name)});
        }
    }
    fire_bound_actions(this, L_, key_name, down);
    process_threads(this);
}

namespace {

bool rect_contains(const UiRect& r, double x, double y) {
    return r.visible && x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height;
}

} // namespace

Runtime::GuiHit Runtime::gui_click(double x, double y, uint32_t viewport_w, uint32_t viewport_h) {
    GuiHit out;
    UiOptions options;
    options.width = viewport_w;
    options.height = viewport_h;
    UiScene scene = build_ui_scene(model_, options);
    // Topmost interactive element: highest render order wins.
    const UiRect* best = nullptr;
    for (const UiRect& rect : scene.rects) {
        if (!rect_contains(rect, x, y)) continue;
        if (rect.class_name != "TextButton" && rect.class_name != "ImageButton" &&
            rect.class_name != "TextBox") {
            continue;
        }
        if (!best || rect.order > best->order) best = &rect;
    }
    if (!best) {
        if (impl_->gui_focused_textbox) {
            fire_instance_signal(this, L_, impl_->gui_focused_textbox, "FocusLost",
                                 {arg_bool(false)});
            impl_->gui_focused_textbox = nullptr;
            process_threads(this);
        }
        return out;
    }
    std::string error;
    Instance* inst = model_.resolve_path(best->path, error);
    if (!inst) return out;
    out.hit = true;
    out.path = best->path;
    out.class_name = best->class_name;
    if (best->class_name == "TextBox") {
        if (impl_->gui_focused_textbox && impl_->gui_focused_textbox != inst) {
            fire_instance_signal(this, L_, impl_->gui_focused_textbox, "FocusLost",
                                 {arg_bool(false)});
        }
        impl_->gui_focused_textbox = inst;
        fire_instance_signal(this, L_, inst, "Focused", {});
        out.event = "Focused";
    } else {
        if (impl_->gui_focused_textbox) {
            fire_instance_signal(this, L_, impl_->gui_focused_textbox, "FocusLost",
                                 {arg_bool(false)});
            impl_->gui_focused_textbox = nullptr;
        }
        fire_instance_signal(this, L_, inst, "MouseButton1Click", {});
        fire_instance_signal(this, L_, inst, "Activated", {});
        out.event = "MouseButton1Click";
    }
    process_threads(this);
    return out;
}

Runtime::GuiHit Runtime::gui_wheel(double x, double y, double delta, uint32_t viewport_w, uint32_t viewport_h) {
    GuiHit out;
    UiOptions options;
    options.width = viewport_w;
    options.height = viewport_h;
    UiScene scene = build_ui_scene(model_, options);
    const UiRect* best = nullptr;
    for (const UiRect& rect : scene.rects) {
        if (!rect_contains(rect, x, y)) continue;
        if (rect.class_name != "ScrollingFrame") continue;
        if (!best || rect.order > best->order) best = &rect;
    }
    if (!best) return out;
    std::string error;
    Instance* inst = model_.resolve_path(best->path, error);
    if (!inst) return out;
    out.hit = true;
    out.path = best->path;
    out.class_name = best->class_name;
    out.event = "InputBegan";
    // Scroll: wheel up (positive delta) moves the canvas up.
    Variant canvas;
    auto it = inst->properties.find("CanvasPosition");
    double cy = 0;
    if (it != inst->properties.end() && it->second.complex_value.is_array() &&
        it->second.complex_value.as_array().size() == 2) {
        cy = it->second.complex_value.as_array()[1].as_double();
    }
    double cx = it != inst->properties.end() && it->second.complex_value.is_array() &&
                        it->second.complex_value.as_array().size() == 2
                    ? it->second.complex_value.as_array()[0].as_double() : 0;
    canvas.type = ValueType::Vector2;
    canvas.complex_value = Json::array();
    canvas.complex_value.push(Json(cx));
    canvas.complex_value.push(Json(std::max(0.0, cy - delta * 40.0)));
    std::string set_error;
    model_.set_property(*inst, "CanvasPosition", canvas, set_error);
    fire_instance_signal(this, L_, inst, "InputBegan", {});
    process_threads(this);
    return out;
}

Runtime::GuiHit Runtime::gui_hover(double x, double y, uint32_t viewport_w, uint32_t viewport_h) {
    GuiHit out;
    UiOptions options;
    options.width = viewport_w;
    options.height = viewport_h;
    UiScene scene = build_ui_scene(model_, options);
    const UiRect* best = nullptr;
    for (const UiRect& rect : scene.rects) {
        if (!rect_contains(rect, x, y)) continue;
        if (!best || rect.order > best->order) best = &rect;
    }
    Instance* inst = nullptr;
    if (best) {
        std::string error;
        inst = model_.resolve_path(best->path, error);
    }
    if (inst == impl_->gui_hovered) {
        if (inst) {
            out.hit = true;
            out.path = model_.path_of(*inst);
            out.class_name = inst->class_name;
        }
        return out;
    }
    if (impl_->gui_hovered) {
        fire_instance_signal(this, L_, impl_->gui_hovered, "MouseLeave", {});
        out.event = "MouseLeave";
    }
    impl_->gui_hovered = inst;
    if (inst) {
        fire_instance_signal(this, L_, inst, "MouseEnter", {});
        out.event = "MouseEnter";
        out.hit = true;
        out.path = model_.path_of(*inst);
        out.class_name = inst->class_name;
    }
    process_threads(this);
    return out;
}

bool Runtime::gui_type_text(const std::string& text, bool submit) {
    Instance* box = impl_->gui_focused_textbox;
    if (!box) return false;
    std::string error;
    Variant value;
    value.type = ValueType::String;
    value.string_value = text;
    if (!model_.set_property(*box, "Text", value, error)) return false;
    fire_changed(this, L_, box, "Text");
    if (submit) {
        fire_instance_signal(this, L_, box, "FocusLost", {arg_bool(true)});
        impl_->gui_focused_textbox = nullptr;
    }
    process_threads(this);
    return true;
}

void Runtime::step(double dt) {
    impl_->step_index++;
    time_ += dt;
    fire_service_signal(this, "RunService", "Stepped", dt);
    process_threads(this);
    // Advance tweens on scheduler time (Phase 27B): delay, then cycles
    // of duration with easing; Reverses alternates direction per cycle,
    // RepeatCount counts additional cycles (-1 repeats forever).
    for (auto& tween : impl_->tweens) {
        if (!tween->playing) continue;
        if (tween->state == 1) { // Delayed
            tween->delay_elapsed += dt;
            if (tween->delay_elapsed < tween->delay_time) continue;
            tween->state = 2;
        }
        tween->elapsed += dt;
        double raw = tween->duration > 0 ? tween->elapsed / tween->duration : 1.0;
        bool cycle_done = raw >= 1.0;
        double alpha = cycle_done ? 1.0 : raw;
        double directed = tween->forward ? alpha : 1.0 - alpha;
        double eased = apply_easing_dir(tween->easing_style, tween->easing_direction, directed);
        for (size_t i = 0; i < tween->goals.size(); ++i) {
            const auto& [prop, goal] = tween->goals[i];
            Variant value = interpolate_variant(tween->starts[i].second, goal, eased);
            std::string error;
            if (model_.set_property(*tween->inst, prop, value, error))
                fire_changed(this, L_, tween->inst, prop);
        }
        if (cycle_done) {
            bool more = tween->repeat_count < 0 || tween->cycle < tween->repeat_count;
            if (more) {
                tween->cycle += 1;
                tween->elapsed = 0;
                if (tween->reverses) tween->forward = !tween->forward;
            } else {
                tween->playing = false;
                tween->state = 4;
                fire_signal(this, L_, tween->completed, {arg_string("Completed")});
            }
        }
    }
    evaluate_anim_tracks(this, dt);
    fire_service_signal(this, "RunService", "Heartbeat", dt);
    process_threads(this);
}


} // namespace blox
