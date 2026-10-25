#pragma once
// Internal seam between the Luau runtime core (runtime.cpp: VM lifecycle,
// scheduler, debugger, Runtime methods) and the Luau bindings
// (bindings.cpp: Instance/datatype/service bindings). Not a public API.
#include <array>
#include <cstdint>
#include <deque>
#include <functional>
#include <initializer_list>
#include <map>
#include <memory>
#include <set>
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

#include "blox/scripting/runtime.hpp"
#include "blox/physics/physics.hpp"
#include "blox/terrain/terrain.hpp"

#include "lua.h"
#include "luacode.h"
#include "lualib.h"

namespace blox {

// ---------------------------------------------------------------- userdata --

enum class UKind { Instance, Vector3, CFrame, Color3, Vector2, UDim, UDim2, EnumItem, Signal, Connection, Tween, RaycastParams, AnimationTrack, PathResult, Region3, DataStore, ColorSequence, NumberSequence, NumberRange, PhysProps, OverlapParams, DataStorePages };

struct SignalImpl;

struct UData {
    UKind kind;
    // RaycastParams
    std::vector<Instance*> filter_list;
    int filter_type = 0; // 0 Exclude / 1 Include
    bool ignore_water = true;
    // Numeric payload: Vector3/Color3/Vector2 xyz; CFrame position[0..2] +
    // rotation row-major[3..11]; UDim scale/offset; UDim2 xs,xo,ys,yo.
    std::array<double, 12> n{};
    std::string s1; // EnumItem: enum type; Connection: unused
    std::string s2; // EnumItem: item name
    Instance* inst = nullptr;
    SignalImpl* signal = nullptr;
    Json complex; // sequence keypoints etc. for sequence kinds
    void* extra = nullptr; // TweenState* for UKind::Tween
    int64_t conn_id = 0;
    bool flag = false; // Connection: still connected
};

struct LuaArg {
    int kind = 0; // 0 nil 1 bool 2 number 3 string 4 instance 5 boxed 6 marshalled-json
    bool b = false;
    double num = 0;
    std::string str;
    Instance* inst = nullptr;
    UData box{};
    std::string json;
};

struct SignalImpl {
    Runtime* rt = nullptr;
    struct Conn {
        int fn_ref = 0;
        bool connected = true;
        bool once = false;
    };
    std::map<int64_t, Conn> conns;
    int64_t next_id = 1;
    std::vector<struct Thread*> waiters;
};

struct Thread {
    lua_State* co = nullptr;
    int co_ref = 0;
    double wake = 0.0;
    double slept_at = 0.0;
    uint64_t ready_step = 0;
    enum class Wait { None, Time, Signal, Debug } wait = Wait::None;
    SignalImpl* waiting_on = nullptr;
    std::string owner;
    std::vector<LuaArg> resume_args;
    int pending_args = -1; // >=0: function + this many args already on the stack
    bool dead = false;
};

struct TweenState {
    Instance* inst = nullptr;
    std::vector<std::pair<std::string, Variant>> goals;
    std::vector<std::pair<std::string, Variant>> starts;
    double duration = 1.0;
    double elapsed = 0.0;
    bool playing = false;
    SignalImpl* completed = nullptr;
    // TweenInfo (Phase 27B).
    std::string easing_style = "Quad";
    std::string easing_direction = "Out";
    int repeat_count = 0; // -1 = repeat forever
    bool reverses = false;
    double delay_time = 0.0;
    double delay_elapsed = 0.0;
    int cycle = 0;
    bool forward = true;
    // 0 Begin, 1 Delayed, 2 Playing, 3 Paused, 4 Completed, 5 Cancelled
    int state = 0;
};


struct Runtime::Impl {
    std::vector<std::unique_ptr<Thread>> threads;
    std::vector<std::unique_ptr<SignalImpl>> signals;
    std::map<std::pair<const Instance*, std::string>, SignalImpl*> instance_signals;
    std::map<std::string, SignalImpl*> service_signals;  // RunService.Heartbeat etc.
    std::map<std::string, SignalImpl*> tag_signals;      // CollectionService per-tag
    std::unordered_set<Instance*> destroyed;
    std::vector<std::unique_ptr<Instance>> detached;  // Instance.new without parent, Destroy()ed
    std::vector<std::unique_ptr<Instance>> graveyard; // keep pointers valid after Destroy
    std::vector<std::unique_ptr<TweenState>> tweens;
    std::unordered_set<const Instance*> modules_loading;
    uint64_t step_index = 0;
    // Phase 9 networking.
    Runtime::NetContext net_context = Runtime::NetContext::Single;
    int player_index = -1;
    Instance* local_player = nullptr;
    std::function<void(const Runtime::RemoteCall&)> remote_sink;
    std::function<Instance*(Instance*)> character_resolver;
    std::map<Instance*, int> remote_invoke_handlers; // RemoteFunction -> fn ref (this runtime)
    std::map<int, Thread*> pending_invokes;
    int next_invoke_id = 1;
    PhysicsWorld* physics = nullptr;
    // Gate 10 animation tracks.
    struct AnimTrackState {
        Instance* animator = nullptr;
        Instance* sequence = nullptr; // KeyframeSequence
        double time_pos = 0;
        double speed = 1.0;
        bool playing = false;
        bool looped = false;
        bool stopped = true; // false once Play() is called, true on Stop()
        bool has_played = false;
        double weight = 1.0;
        std::vector<Instance*> driven; // Motor6Ds this track last wrote
    };
    std::vector<std::unique_ptr<AnimTrackState>> anim_tracks;
    // Phase 23 debugger state.
    bool debug_mode = false;
    std::map<std::string, std::set<int>> debug_breakpoints; // script path -> lines
    Thread* debug_paused_thread = nullptr;
    // Phase 22 GUI pointer state.
    Instance* gui_focused_textbox = nullptr;
    Instance* gui_hovered = nullptr;
    std::string debug_pause_reason;
    int debug_step_mode = 0; // 0 none, 1 into, 2 over, 3 out
    int debug_step_depth = 0;
    int debug_step_line = -1;
    bool debug_step_hit = false;
    bool debug_break_hit = false;
    int debug_pause_line = 0;
    std::string debug_pause_script;
    int debug_reenable_line = 0;
    int debug_reenable_ref = 0;
    Thread* debug_reenable_thread = nullptr;
    Thread* debug_paused_candidate = nullptr;
    // PathfindingService paths (Phase 14).
    struct PathState {
        std::vector<std::array<double, 3>> waypoints;
        std::vector<int> actions; // 0 Walk, 1 Jump
        std::string status = "NoPath";
        double agent_radius = 2.0;
        bool can_jump = true;
    };
    std::vector<std::unique_ptr<PathState>> paths;
    std::map<Instance*, std::string> humanoid_states;
    // Phase 29 offline DataStores: "name/scope" -> key -> JSON value.
    std::string datastore_dir;
    std::map<std::string, std::map<std::string, Json>> datastores;
    bool datastores_loaded = false;
    // Terrain CopyRegion clipboard (Phase 26): relative voxel cells.
    std::map<Instance*, std::map<std::tuple<int, int, int>, std::pair<uint16_t, uint8_t>>> terrain_clipboard;
    // ContextActionService bindings (Phase 14).
    struct BoundAction {
        std::string name;
        int fn_ref = -1;
        std::vector<std::string> inputs; // KeyCode names, e.g. "E"
    };
    std::vector<BoundAction> bound_actions;
    std::map<void*, std::vector<Instance*>> raycast_filters;
    uint64_t next_referent = 2'000'000'000ULL;
    uint64_t interrupts = 0;
    uint64_t interrupt_budget = 2'000'000;
    std::string current_owner;
    Instance* staging = nullptr; // hidden holder used to mint defaulted instances
    std::map<std::string, bool> key_states; // UserInputService (play mode feeds this)

    SignalImpl* new_signal(Runtime* rt) {
        auto sig = std::make_unique<SignalImpl>();
        sig->rt = rt;
        SignalImpl* raw = sig.get();
        signals.push_back(std::move(sig));
        return raw;
    }
    SignalImpl* instance_signal(Runtime* rt, const Instance* inst, const std::string& key) {
        auto it = instance_signals.find({inst, key});
        if (it != instance_signals.end()) return it->second;
        SignalImpl* raw = new_signal(rt);
        instance_signals[{inst, key}] = raw;
        return raw;
    }
    SignalImpl* find_instance_signal(const Instance* inst, const std::string& key) const {
        auto it = instance_signals.find({inst, key});
        return it == instance_signals.end() ? nullptr : it->second;
    }
};

// Shared binding/core functions (defined in bindings.cpp or runtime.cpp).
LuaArg enum_arg(const char* type, const std::string& name);
UData* new_box(lua_State* L, UKind kind, const char* metatable);
UData* check_box(lua_State* L, int idx, UKind kind, const char* what);
void push_vector3(lua_State* L, double x, double y, double z);
void push_vector2(lua_State* L, double x, double y);
void push_color3(lua_State* L, double r, double g, double b);
void push_cframe(lua_State* L, const Variant& cf);
void push_udim(lua_State* L, double scale, double offset);
void push_udim2(lua_State* L, double xs, double xo, double ys, double yo);
void push_enum_item(lua_State* L, const std::string& type, const std::string& name, int64_t value);
void push_signal(lua_State* L, SignalImpl* sig);
const char* metatable_for(UKind kind);
void push_args(lua_State* L, const std::vector<LuaArg>& args);
LuaArg arg_json(const std::string& json);
Instance* find_by_referent(DataModel& model, uint64_t referent);
Json lua_value_to_json(lua_State* L, int idx, int depth, std::string& error);
Json lua_args_to_json(lua_State* L, int first, std::string& error);
void push_json_value(lua_State* L, Runtime* rt, const Json& j);
Json json_arr(std::initializer_list<Json> items);
LuaArg arg_number(double v);
LuaArg arg_bool(bool v);
LuaArg arg_string(const std::string& v);
LuaArg arg_instance(Instance* v);
Instance* resolve_ref(DataModel& model, const std::string& path);
bool variant_to_lua(lua_State* L, Runtime* rt, const Variant& v, const std::string& prop_name);
bool lua_to_variant(lua_State* L, int idx, ValueType type, Variant& out, std::string& error);
bool lua_to_variant_inferred(lua_State* L, int idx, Variant& out, std::string& error);
std::string traceback_of(lua_State* co, const std::string& message);
void push_instance(lua_State* L, Instance* inst);
Instance* check_instance(lua_State* L, int idx);
bool in_game(Runtime* rt, const Instance* inst);
void check_alive(Runtime* rt, Instance* inst);
std::unique_ptr<Instance> extract_owned(Runtime* rt, Instance* inst);
void fire_instance_signal(Runtime* rt, lua_State* L, Instance* inst, const std::string& key,
                          const std::vector<LuaArg>& args);
void fire_changed(Runtime* rt, lua_State* L, Instance* inst, const std::string& property);
Instance* mint_instance(Runtime* rt, const std::string& class_name, const std::string& name);
void adopt_into(Runtime* rt, Instance* inst, Instance* new_parent);
int inst_index(lua_State* L);
int inst_newindex(lua_State* L);
int inst_eq(lua_State* L);
int inst_tostring(lua_State* L);
int m_find_first_child(lua_State* L);
int m_find_first_child_of_class(lua_State* L);
int m_get_children(lua_State* L);
int m_get_descendants(lua_State* L);
bool class_is_a(const std::string& cls, const std::string& query);
int m_is_a(lua_State* L);
int m_clone(lua_State* L);
int m_destroy(lua_State* L);
int m_clear_children(lua_State* L);
int m_get_full_name(lua_State* L);
int m_get_service(lua_State* L);
Variant lua_value_to_attribute(lua_State* L, int idx, bool& ok);
int m_get_attribute(lua_State* L);
int m_set_attribute(lua_State* L);
int m_get_property_changed_signal(lua_State* L);
std::vector<std::string> tags_of(Instance* inst);
void set_tags(Runtime* rt, lua_State* L, Instance* inst, const std::vector<std::string>& tags);
int m_add_tag(lua_State* L);
int m_remove_tag(lua_State* L);
int m_has_tag(lua_State* L);
int m_get_tags(lua_State* L);
int sig_connect_impl(lua_State* L, bool once);
int sig_connect(lua_State* L);
int sig_once(lua_State* L);
int sig_wait(lua_State* L);
int conn_index(lua_State* L);
int conn_disconnect(lua_State* L);
double apply_easing(const std::string& style, double t);
Variant interpolate_variant(const Variant& from, const Variant& to, double a);
int tween_play(lua_State* L);
int tween_pause(lua_State* L);
int tween_cancel(lua_State* L);
int tween_index(lua_State* L);
int m_tween_create(lua_State* L);
int m_collection_add_tag(lua_State* L);
int m_collection_remove_tag(lua_State* L);
int m_collection_has_tag(lua_State* L);
int m_collection_get_tagged(lua_State* L);
int m_collection_tag_signal(lua_State* L);
int m_players_get_players(lua_State* L);
int m_runservice_is_running(lua_State* L);
int m_runservice_is_server(lua_State* L);
int m_runservice_is_client(lua_State* L);
int m_unsupported(lua_State* L);
int m_uis_is_key_down(lua_State* L);
int task_wait(lua_State* L);
int task_spawn(lua_State* L);
int task_defer(lua_State* L);
int task_delay(lua_State* L);
int task_clock(lua_State* L);
int fn_instance_new(lua_State* L);
int fn_require(lua_State* L);
Instance* check_remote(lua_State* L, int idx, const char* klass);
int m_remote_fire_server(lua_State* L);
int m_remote_fire_client(lua_State* L);
int m_remote_fire_all_clients(lua_State* L);
int m_remote_invoke_impl(lua_State* L, bool to_server);
int m_remote_invoke_server(lua_State* L);
int m_remote_invoke_client(lua_State* L);
int m_workspace_get_parts_in_box(lua_State* L);
int m_workspace_get_parts_in_radius(lua_State* L);
int m_workspace_raycast(lua_State* L);
int m_raycast_params_new(lua_State* L);
int rcp_index(lua_State* L);
int rcp_newindex(lua_State* L);
Variant sample_sequence(Instance* sequence, double t, std::map<std::string, Variant>& out);
Instance* find_motor6d(Instance* root, const std::string& joint_name);
Instance* model_of(Instance* inst);
double sequence_length(Instance* sequence);
int m_cas_bind_action(lua_State* L);
int m_cas_unbind_action(lua_State* L);
int m_cas_unbind_all(lua_State* L);
void fire_bound_actions(Runtime* rt, lua_State* L, const std::string& key_name, bool down);
void set_service_attribute(Runtime* rt, Instance* inst, const std::string& key,
                           const std::string& type, const Json& value);
bool get_service_bool_attribute(Instance* inst, const std::string& key, bool fallback);
Instance* check_physics_service(lua_State* L, int idx);
int m_physics_register_group(lua_State* L);
int m_physics_set_part_group(lua_State* L);
std::string collision_pair_key(const std::string& a, const std::string& b);
int m_physics_set_collidable(lua_State* L);
int m_physics_are_collidable(lua_State* L);
int m_pathfinding_create_path(lua_State* L);
int m_path_compute(lua_State* L);
int m_path_get_waypoints(lua_State* L);
int path_index(lua_State* L);
int m_humanoid_get_state(lua_State* L);
int m_humanoid_change_state(lua_State* L);
int m_animator_load_animation(lua_State* L);
int track_play(lua_State* L);
int track_stop(lua_State* L);
int track_adjust_weight(lua_State* L);
int track_adjust_speed(lua_State* L);
int track_index(lua_State* L);
int track_newindex(lua_State* L);
void evaluate_anim_tracks(Runtime* rt, double dt);
Instance* check_terrain(lua_State* L, int idx);
uint16_t material_from_enum(lua_State* L, int idx);
int m_terrain_fill_block(lua_State* L);
int m_terrain_fill_cylinder(lua_State* L);
int m_terrain_fill_wedge(lua_State* L);
int m_terrain_fill_region(lua_State* L);
int m_terrain_write_voxels(lua_State* L);
int m_terrain_copy_region(lua_State* L);
int m_terrain_paste_region(lua_State* L);
int region3_new(lua_State* L);
int region3_expand(lua_State* L);
void push_region3(lua_State* L, std::array<double, 3> mn, std::array<double, 3> mx);
int region3_index(lua_State* L);
int m_terrain_fill_ball(lua_State* L);
int m_terrain_clear(lua_State* L);
int m_terrain_read_voxels(lua_State* L);
int m_text_service_get_text_size(lua_State* L);
int fn_typeof(lua_State* L);
int fn_tween_info_new(lua_State* L);
int cs_new(lua_State* L);
int ns_new(lua_State* L);
int nr_new(lua_State* L);
int pp_new(lua_State* L);
int op_new(lua_State* L);
int cs_keypoint_new(lua_State* L);
int ns_keypoint_new(lua_State* L);
int sequence_index(lua_State* L);
int nr_index(lua_State* L);
int pp_index(lua_State* L);
int op_index(lua_State* L);
int op_newindex(lua_State* L);
void push_sequence(lua_State* L, UKind kind, const Json& complex);
int m_datastore_get_store(lua_State* L);
int m_datastore_get_ordered(lua_State* L);
int ds_get_async(lua_State* L);
int ds_set_async(lua_State* L);
int ds_update_async(lua_State* L);
int ds_increment_async(lua_State* L);
int ds_remove_async(lua_State* L);
int ds_get_sorted(lua_State* L);
int dsp_index(lua_State* L);
int m_datastore_budget(lua_State* L);
int m_http_json_encode(lua_State* L);
int m_http_json_decode(lua_State* L);
int m_http_generate_guid(lua_State* L);
int m_http_request(lua_State* L);
int ds_index(lua_State* L);
void datastore_load(Runtime* rt);
void datastore_save(Runtime* rt);
double apply_easing_dir(const std::string& style, const std::string& direction, double t);
void log_args(lua_State* L, const char* level);
int fn_print(lua_State* L);
int fn_warn(lua_State* L);
int fn_tick(lua_State* L);
int v3_index(lua_State* L);
int v3_lerp(lua_State* L);
int v3_dot(lua_State* L);
int v3_cross(lua_State* L);
int v3_add(lua_State* L);
int v3_sub(lua_State* L);
int v3_mul(lua_State* L);
int v3_div(lua_State* L);
int v3_unm(lua_State* L);
int v3_eq(lua_State* L);
int v3_new(lua_State* L);
int cf_index(lua_State* L);
int cf_components(lua_State* L);
int cf_mul(lua_State* L);
int cf_new(lua_State* L);
int cf_look_at(lua_State* L);
int cf_angles(lua_State* L);
int cf_eq(lua_State* L);
int c3_index(lua_State* L);
int c3_new(lua_State* L);
int c3_from_rgb(lua_State* L);
int c3_eq(lua_State* L);
int v2_index(lua_State* L);
int v2_new(lua_State* L);
int udim_index(lua_State* L);
int udim_new(lua_State* L);
int udim2_index(lua_State* L);
int udim2_new(lua_State* L);
int udim2_from_scale(lua_State* L);
int udim2_from_offset(lua_State* L);
int enumitem_index(lua_State* L);
int enumitem_eq(lua_State* L);
Runtime* rt_of(lua_State* L);
Thread* make_thread(Runtime* rt, lua_State* L, const std::string& owner);
void kill_thread(Runtime* rt, lua_State* L, Thread* t);
void fire_signal(Runtime* rt, lua_State* L, SignalImpl* sig, const std::vector<LuaArg>& args);
void debug_break_hook(lua_State* L, lua_Debug*);
void debug_step_hook(lua_State* L, lua_Debug*);
void debug_reenable_breakpoint_impl(Runtime* rt, Thread* t);
void debug_reenable_breakpoint(Runtime* rt, Thread* t);
std::string debug_format_value(lua_State* L, int idx);
void register_bindings(lua_State* L, Runtime* rt);
bool load_chunk(lua_State* co, const std::string& source, const std::string& chunk_name, std::string& error);

} // namespace blox
