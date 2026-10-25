// Luau bindings: userdata, marshalling, Instance methods, signals,
// tweens, services, remotes, animation, pathfinding, terrain/TextService
// and datatype bindings. The scheduler/debugger and Runtime lifecycle
// live in runtime.cpp; shared declarations are in runtime_internal.hpp.
#include "runtime_internal.hpp"
#include "blox/terrain/terrain.hpp"
#include "blox/gui/ui.hpp"

#include <algorithm>
#include <cmath>
#include <cstring>

namespace blox {

// ---------------------------------------------------------------- helpers --

UData* new_box(lua_State* L, UKind kind, const char* metatable) {
    void* mem = lua_newuserdata(L, sizeof(UData));
    auto* u = new (mem) UData();
    u->kind = kind;
    luaL_getmetatable(L, metatable);
    lua_setmetatable(L, -2);
    return u;
}

UData* check_box(lua_State* L, int idx, UKind kind, const char* what) {
    void* mem = lua_touserdata(L, idx);
    if (!mem) luaL_error(L, "expected %s", what);
    auto* u = static_cast<UData*>(mem);
    if (u->kind != kind) luaL_error(L, "expected %s", what);
    return u;
}

void push_vector3(lua_State* L, double x, double y, double z) {
    UData* u = new_box(L, UKind::Vector3, "BloxVector3");
    u->n = {x, y, z};
}
void push_vector2(lua_State* L, double x, double y) {
    UData* u = new_box(L, UKind::Vector2, "BloxVector2");
    u->n = {x, y, 0};
}
void push_color3(lua_State* L, double r, double g, double b) {
    UData* u = new_box(L, UKind::Color3, "BloxColor3");
    u->n = {r, g, b};
}
void push_cframe(lua_State* L, const Variant& cf) {
    UData* u = new_box(L, UKind::CFrame, "BloxCFrame");
    u->n = {cf.position[0], cf.position[1], cf.position[2],
            cf.rotation[0], cf.rotation[1], cf.rotation[2], cf.rotation[3], cf.rotation[4],
            cf.rotation[5], cf.rotation[6], cf.rotation[7], cf.rotation[8]};
}
void push_udim(lua_State* L, double scale, double offset) {
    UData* u = new_box(L, UKind::UDim, "BloxUDim");
    u->n = {scale, offset, 0};
}
void push_udim2(lua_State* L, double xs, double xo, double ys, double yo) {
    UData* u = new_box(L, UKind::UDim2, "BloxUDim2");
    u->n = {xs, xo, ys, yo};
}
void push_enum_item(lua_State* L, const std::string& type, const std::string& name, int64_t value) {
    UData* u = new_box(L, UKind::EnumItem, "BloxEnumItem");
    u->s1 = type;
    u->s2 = name;
    u->n[0] = static_cast<double>(value);
}
void push_signal(lua_State* L, SignalImpl* sig) {
    UData* u = new_box(L, UKind::Signal, "BloxSignal");
    u->signal = sig;
}

void push_instance(lua_State* L, Instance* inst); // fwd

const char* metatable_for(UKind kind) {
    switch (kind) {
        case UKind::Vector3: return "BloxVector3";
        case UKind::CFrame: return "BloxCFrame";
        case UKind::Color3: return "BloxColor3";
        case UKind::Vector2: return "BloxVector2";
        case UKind::UDim: return "BloxUDim";
        case UKind::UDim2: return "BloxUDim2";
        case UKind::EnumItem: return "BloxEnumItem";
        case UKind::Region3: return "BloxRegion3";
        default: return "BloxVector3";
    }
}

void push_json_value(lua_State* L, Runtime* rt, const Json& j); // fwd

void push_args(lua_State* L, const std::vector<LuaArg>& args) {
    Runtime* rt = rt_of(L);
    for (const LuaArg& a : args) {
        switch (a.kind) {
            case 1: lua_pushboolean(L, a.b ? 1 : 0); break;
            case 2: lua_pushnumber(L, a.num); break;
            case 3: lua_pushlstring(L, a.str.data(), a.str.size()); break;
            case 4: push_instance(L, a.inst); break;
            case 5: {
                UData* u = new_box(L, a.box.kind, metatable_for(a.box.kind));
                *u = a.box;
                break;
            }
            case 6: {
                Json parsed;
                std::string parse_error;
                if (Json::parse(a.json, parsed, parse_error)) push_json_value(L, rt, parsed);
                else lua_pushnil(L);
                break;
            }
            default: lua_pushnil(L); break;
        }
    }
}

LuaArg arg_json(const std::string& json) {
    LuaArg a;
    a.kind = 6;
    a.json = json;
    return a;
}

// ------------------------------------------------------------ marshalling --
// Lua values crossing a (simulated) network boundary are converted to
// JSON: primitives and plain tables directly; datatypes and instances
// as tagged objects. Functions/userdata of other kinds are rejected.

Instance* find_by_referent(DataModel& model, uint64_t referent) {
    std::vector<Instance*> stack{&model.root()};
    while (!stack.empty()) {
        Instance* cur = stack.back();
        stack.pop_back();
        if (cur->referent == referent) return cur;
        for (const auto& child : cur->children) stack.push_back(child.get());
    }
    return nullptr;
}

Json lua_value_to_json(lua_State* L, int idx, int depth, std::string& error) {
    if (depth > 8) { error = "value too deeply nested to send over a RemoteEvent"; return Json(); }
    int type = lua_type(L, idx);
    if (type == LUA_TNIL) return Json();
    if (type == LUA_TBOOLEAN) return Json(lua_toboolean(L, idx) != 0);
    if (type == LUA_TNUMBER) return Json(lua_tonumber(L, idx));
    if (type == LUA_TSTRING) {
        size_t len = 0;
        const char* str = lua_tolstring(L, idx, &len);
        return Json(std::string(str, len));
    }
    if (type == LUA_TUSERDATA) {
        auto* u = static_cast<UData*>(lua_touserdata(L, idx));
        Json tagged = Json::object();
        switch (u->kind) {
            case UKind::Instance: {
                tagged.set("$t", Json("Instance"));
                Runtime* owner = rt_of(L);
                if (owner) tagged.set("p", Json(owner->model().path_of(*u->inst)));
                tagged.set("r", Json(static_cast<int64_t>(u->inst->referent)));
                return tagged;
            }
            case UKind::Vector3: case UKind::Color3: case UKind::Vector2: {
                tagged.set("$t", Json(u->kind == UKind::Vector3 ? "Vector3"
                                      : u->kind == UKind::Color3 ? "Color3" : "Vector2"));
                Json v = Json::array();
                int n = u->kind == UKind::Vector2 ? 2 : 3;
                for (int i = 0; i < n; ++i) v.push(Json(u->n[i]));
                tagged.set("v", std::move(v));
                return tagged;
            }
            case UKind::CFrame: {
                tagged.set("$t", Json("CFrame"));
                Json v = Json::array();
                for (int i = 0; i < 12; ++i) v.push(Json(u->n[i]));
                tagged.set("v", std::move(v));
                return tagged;
            }
            case UKind::UDim: case UKind::UDim2: {
                tagged.set("$t", Json(u->kind == UKind::UDim ? "UDim" : "UDim2"));
                Json v = Json::array();
                int n = u->kind == UKind::UDim ? 2 : 4;
                for (int i = 0; i < n; ++i) v.push(Json(u->n[i]));
                tagged.set("v", std::move(v));
                return tagged;
            }
            case UKind::EnumItem: {
                tagged.set("$t", Json("EnumItem"));
                tagged.set("e", Json(u->s1));
                tagged.set("n", Json(u->s2));
                tagged.set("v", Json(u->n[0]));
                return tagged;
            }
            default:
                error = "cannot send this userdata type over a RemoteEvent";
                return Json();
        }
    }
    if (type == LUA_TTABLE) {
        // Array if keys are 1..n, else object with string keys.
        bool is_array = true;
        lua_pushnil(L);
        int count = 0;
        while (lua_next(L, idx) != 0) {
            ++count;
            if (!lua_isnumber(L, -2) || lua_tonumber(L, -2) != count) is_array = false;
            lua_pop(L, 1);
        }
        if (is_array) {
            Json arr = Json::array();
            for (int i = 1; i <= count; ++i) {
                lua_rawgeti(L, idx, i);
                Json v = lua_value_to_json(L, lua_gettop(L), depth + 1, error);
                lua_pop(L, 1);
                if (!error.empty()) return Json();
                arr.push(std::move(v));
            }
            return arr;
        }
        Json obj = Json::object();
        lua_pushnil(L);
        while (lua_next(L, idx) != 0) {
            if (!lua_isstring(L, -2)) { error = "table keys must be strings to send over a RemoteEvent"; lua_pop(L, 2); return Json(); }
            std::string key = lua_tostring(L, -2);
            Json v = lua_value_to_json(L, lua_gettop(L), depth + 1, error);
            lua_pop(L, 1);
            if (!error.empty()) { lua_pop(L, 1); return Json(); }
            obj.set(key, std::move(v));
        }
        return obj;
    }
    error = "cannot send values of this type over a RemoteEvent";
    return Json();
}

Json lua_args_to_json(lua_State* L, int first, std::string& error) {
    Json arr = Json::array();
    for (int i = first; i <= lua_gettop(L); ++i) {
        Json v = lua_value_to_json(L, i, 0, error);
        if (!error.empty()) return Json();
        arr.push(std::move(v));
    }
    return arr;
}

void push_json_value(lua_State* L, Runtime* rt, const Json& j) {
    if (j.is_null()) { lua_pushnil(L); return; }
    if (j.is_bool()) { lua_pushboolean(L, j.as_bool() ? 1 : 0); return; }
    if (j.is_number()) { lua_pushnumber(L, j.as_double()); return; }
    if (j.is_string()) { lua_pushlstring(L, j.as_string().data(), j.as_string().size()); return; }
    if (j.is_array()) {
        lua_newtable(L);
        int i = 1;
        for (const Json& v : j.as_array()) { push_json_value(L, rt, v); lua_rawseti(L, -2, i++); }
        return;
    }
    if (j.is_object()) {
        if (const Json* tag = j.find("$t")) {
            std::string kind = tag->as_string();
            const Json* v = j.find("v");
            if (kind == "Instance") {
                Instance* inst = nullptr;
                if (rt) {
                    if (const Json* path = j.find("p")) {
                        std::string err;
                        inst = rt->model().resolve_path(path->as_string(), err);
                    }
                    if (!inst)
                        inst = find_by_referent(rt->model(), static_cast<uint64_t>(j.find("r") ? j.find("r")->as_int() : 0));
                }
                push_instance(L, inst);
                return;
            }
            if (v && v->is_array()) {
                const auto& a = v->as_array();
                auto at = [&](size_t i) { return i < a.size() ? a[i].as_double() : 0.0; };
                if (kind == "Vector3") { push_vector3(L, at(0), at(1), at(2)); return; }
                if (kind == "Color3") { push_color3(L, at(0), at(1), at(2)); return; }
                if (kind == "Vector2") { push_vector2(L, at(0), at(1)); return; }
                if (kind == "CFrame") {
                    UData* u = new_box(L, UKind::CFrame, "BloxCFrame");
                    for (int i = 0; i < 12; ++i) u->n[i] = at(static_cast<size_t>(i));
                    return;
                }
                if (kind == "UDim") { push_udim(L, at(0), at(1)); return; }
                if (kind == "UDim2") { push_udim2(L, at(0), at(1), at(2), at(3)); return; }
            }
            if (kind == "EnumItem") {
                push_enum_item(L, j.find("e") ? j.find("e")->as_string() : "",
                               j.find("n") ? j.find("n")->as_string() : "",
                               j.find("v") ? static_cast<int64_t>(j.find("v")->as_int()) : 0);
                return;
            }
        }
        lua_newtable(L);
        for (const auto& [key, value] : j.as_object()) {
            push_json_value(L, rt, value);
            lua_setfield(L, -2, key.c_str());
        }
        return;
    }
    lua_pushnil(L);
}

Json json_arr(std::initializer_list<Json> items) {
    Json out = Json::array();
    for (const Json& item : items) {
        Json copy = item;
        out.push(std::move(copy));
    }
    return out;
}

LuaArg arg_number(double v) { LuaArg a; a.kind = 2; a.num = v; return a; }
LuaArg arg_bool(bool v) { LuaArg a; a.kind = 1; a.b = v; return a; }
LuaArg arg_string(const std::string& v) { LuaArg a; a.kind = 3; a.str = v; return a; }
LuaArg arg_instance(Instance* v) { LuaArg a; a.kind = 4; a.inst = v; return a; }

// Resolve a Ref value ("0/2/1" child-index path from the root, or a
// "name:Workspace/Part/Child" name path used for cross-tree refs).
Instance* resolve_ref(DataModel& model, const std::string& path) {
    if (path.empty()) return nullptr;
    if (path.rfind("name:", 0) == 0) {
        Instance* cur = &model.root();
        size_t at = 5;
        while (at <= path.size()) {
            size_t slash = path.find('/', at);
            std::string part = path.substr(at, slash == std::string::npos ? slash : slash - at);
            Instance* next = nullptr;
            for (const auto& child : cur->children)
                if (child->name == part) { next = child.get(); break; }
            if (!next) return nullptr;
            cur = next;
            if (slash == std::string::npos) break;
            at = slash + 1;
        }
        return cur;
    }
    Instance* cur = &model.root();
    size_t at = 0;
    while (at <= path.size()) {
        size_t slash = path.find('/', at);
        std::string part = path.substr(at, slash == std::string::npos ? slash : slash - at);
        char* end = nullptr;
        long idx = std::strtol(part.c_str(), &end, 10);
        if (end == part.c_str() || idx < 0 || static_cast<size_t>(idx) >= cur->children.size()) return nullptr;
        cur = cur->children[static_cast<size_t>(idx)].get();
        if (slash == std::string::npos) break;
        at = slash + 1;
    }
    return cur;
}

// ------------------------------------------------------------ variant <-> lua --

bool variant_to_lua(lua_State* L, Runtime* rt, const Variant& v, const std::string& prop_name) {
    switch (v.type) {
        case ValueType::Bool: lua_pushboolean(L, v.bool_value ? 1 : 0); return true;
        case ValueType::Int32: case ValueType::Int64:
        case ValueType::Faces: case ValueType::Axes:
            lua_pushnumber(L, static_cast<double>(v.int_value)); return true;
        case ValueType::Float32: case ValueType::Float64:
            lua_pushnumber(L, v.float_value); return true;
        case ValueType::String: case ValueType::ContentId:
            lua_pushlstring(L, v.string_value.data(), v.string_value.size()); return true;
        case ValueType::BinaryString:
            lua_pushlstring(L, v.string_value.data(), v.string_value.size()); return true;
        case ValueType::Vector3: push_vector3(L, v.vec3[0], v.vec3[1], v.vec3[2]); return true;
        case ValueType::Color3: push_color3(L, v.vec3[0], v.vec3[1], v.vec3[2]); return true;
        case ValueType::Vector2:
            if (v.complex_value.is_array() && v.complex_value.as_array().size() == 2)
                push_vector2(L, v.complex_value.as_array()[0].as_double(), v.complex_value.as_array()[1].as_double());
            else push_vector2(L, 0, 0);
            return true;
        case ValueType::CFrame: push_cframe(L, v); return true;
        case ValueType::Enum: {
            // Map well-known enum-typed properties; others surface as numbers.
            std::string type = prop_name == "Material"    ? "Material"
                               : prop_name == "Shape"     ? "PartType"
                               : prop_name == "RigType"   ? "HumanoidRigType"
                                                          : "";
            if (!type.empty()) {
                lua_getfield(L, LUA_REGISTRYINDEX, "blox_enum_items");
                lua_pushstring(L, (type + ":" + std::to_string(v.int_value)).c_str());
                lua_rawget(L, -2);
                if (!lua_isnil(L, -1)) { lua_remove(L, -2); return true; }
                lua_pop(L, 2);
                push_enum_item(L, type, "", v.int_value);
                return true;
            }
            lua_pushnumber(L, static_cast<double>(v.int_value));
            return true;
        }
        case ValueType::EnumItem: {
            if (v.complex_value.is_object()) {
                push_enum_item(L, v.complex_value.find("enum") ? v.complex_value.find("enum")->as_string() : "",
                               "", static_cast<int64_t>(v.complex_value.find("value") ? v.complex_value.find("value")->as_double() : 0));
                return true;
            }
            lua_pushnil(L); return true;
        }
        case ValueType::BrickColor: lua_pushnumber(L, static_cast<double>(v.int_value)); return true;
        case ValueType::UDim: {
            const auto& a = v.complex_value.as_array();
            if (v.complex_value.is_array() && a.size() == 2) push_udim(L, a[0].as_double(), a[1].as_double());
            else push_udim(L, 0, 0);
            return true;
        }
        case ValueType::UDim2: {
            const auto& a = v.complex_value.as_array();
            if (v.complex_value.is_array() && a.size() == 4)
                push_udim2(L, a[0].as_double(), a[1].as_double(), a[2].as_double(), a[3].as_double());
            else push_udim2(L, 0, 0, 0, 0);
            return true;
        }
        case ValueType::Tags: {
            lua_newtable(L);
            if (v.complex_value.is_array()) {
                int i = 1;
                for (const Json& t : v.complex_value.as_array()) { lua_pushstring(L, t.as_string().c_str()); lua_rawseti(L, -2, i++); }
            }
            return true;
        }
        case ValueType::ColorSequence: case ValueType::NumberSequence: {
            lua_newtable(L);
            if (v.complex_value.is_array()) {
                int i = 1;
                for (const Json& kp : v.complex_value.as_array()) {
                    lua_newtable(L);
                    lua_pushnumber(L, kp.find("time") ? kp.find("time")->as_double() : 0);
                    lua_setfield(L, -2, "Time");
                    if (v.type == ValueType::ColorSequence && kp.find("color")) {
                        const auto& c = kp.find("color")->as_array();
                        push_color3(L, c[0].as_double(), c[1].as_double(), c[2].as_double());
                        lua_setfield(L, -2, "Color");
                    } else {
                        lua_pushnumber(L, kp.find("value") ? kp.find("value")->as_double() : 0);
                        lua_setfield(L, -2, "Value");
                        lua_pushnumber(L, kp.find("envelope") ? kp.find("envelope")->as_double() : 0);
                        lua_setfield(L, -2, "Envelope");
                    }
                    lua_rawseti(L, -2, i++);
                }
            }
            return true;
        }
        case ValueType::Ref: {
            Instance* target = resolve_ref(rt->model(), v.string_value);
            if (target) push_instance(L, target); else lua_pushnil(L);
            return true;
        }
        case ValueType::Font: {
            lua_newtable(L);
            if (v.complex_value.is_object()) {
                lua_pushstring(L, v.complex_value.find("family") ? v.complex_value.find("family")->as_string().c_str() : "");
                lua_setfield(L, -2, "Family");
                lua_pushnumber(L, v.complex_value.find("weight") ? v.complex_value.find("weight")->as_double() : 400);
                lua_setfield(L, -2, "Weight");
            }
            return true;
        }
        case ValueType::NumberRange: {
            lua_newtable(L);
            if (v.complex_value.is_array() && v.complex_value.as_array().size() == 2) {
                lua_pushnumber(L, v.complex_value.as_array()[0].as_double()); lua_setfield(L, -2, "Min");
                lua_pushnumber(L, v.complex_value.as_array()[1].as_double()); lua_setfield(L, -2, "Max");
            }
            return true;
        }
        default:
            lua_pushnil(L);
            return true;
    }
}

// Convert a Lua value at `idx` into a Variant of the requested type.
bool lua_to_variant(lua_State* L, int idx, ValueType type, Variant& out, std::string& error) {
    switch (type) {
        case ValueType::Bool: out = Variant::make_bool(lua_toboolean(L, idx) != 0); return true;
        case ValueType::Int32: case ValueType::Int64:
        case ValueType::Faces: case ValueType::Axes:
            out = Variant::make_int64(static_cast<int64_t>(luaL_checknumber(L, idx))); return true;
        case ValueType::Float32: case ValueType::Float64:
            out = Variant::make_float64(luaL_checknumber(L, idx)); return true;
        case ValueType::String: case ValueType::ContentId: {
            size_t len = 0;
            const char* s = luaL_checklstring(L, idx, &len);
            out = Variant::make_string(std::string(s, len));
            if (type == ValueType::ContentId) out.type = ValueType::ContentId;
            return true;
        }
        case ValueType::Vector3: {
            auto* u = check_box(L, idx, UKind::Vector3, "Vector3");
            out = Variant::make_vec3(u->n[0], u->n[1], u->n[2]); return true;
        }
        case ValueType::Color3: {
            auto* u = check_box(L, idx, UKind::Color3, "Color3");
            out = Variant::make_color3(u->n[0], u->n[1], u->n[2]); return true;
        }
        case ValueType::Vector2: {
            auto* u = check_box(L, idx, UKind::Vector2, "Vector2");
            out.type = ValueType::Vector2;
            out.complex_value = json_arr({Json(u->n[0]), Json(u->n[1])});
            return true;
        }
        case ValueType::CFrame: {
            auto* u = check_box(L, idx, UKind::CFrame, "CFrame");
            out.type = ValueType::CFrame;
            out.position = {u->n[0], u->n[1], u->n[2]};
            for (int i = 0; i < 9; ++i) out.rotation[i] = u->n[3 + i];
            return true;
        }
        case ValueType::Enum: {
            out.type = ValueType::Enum;
            if (lua_isnumber(L, idx)) out.int_value = static_cast<int64_t>(lua_tonumber(L, idx));
            else {
                auto* u = check_box(L, idx, UKind::EnumItem, "EnumItem");
                out.int_value = static_cast<int64_t>(u->n[0]);
            }
            return true;
        }
        case ValueType::EnumItem: {
            auto* u = check_box(L, idx, UKind::EnumItem, "EnumItem");
            out.type = ValueType::EnumItem;
            out.complex_value = Json::object();
            out.complex_value.set("enum", Json(u->s1));
            out.complex_value.set("value", Json(u->n[0]));
            return true;
        }
        case ValueType::BrickColor: {
            out.type = ValueType::BrickColor;
            out.int_value = static_cast<int64_t>(luaL_checknumber(L, idx));
            return true;
        }
        case ValueType::Ref: {
            if (lua_isnil(L, idx)) { out.type = ValueType::Ref; out.string_value = ""; return true; }
            auto* u = static_cast<UData*>(lua_touserdata(L, idx));
            if (!u || u->kind != UKind::Instance || !u->inst) { error = "expected an Instance for a Ref property"; return false; }
            // Child-index path from the root ("0/2/1").
            std::string path;
            Instance* cur = u->inst;
            DataModel& model = rt_of(L)->model();
            while (cur && cur != &model.root()) {
                Instance* parent = cur->parent;
                if (!parent) { error = "instance is not in the DataModel"; return false; }
                size_t index = 0;
                bool found = false;
                for (size_t i = 0; i < parent->children.size(); ++i) {
                    if (parent->children[i].get() == cur) { index = i; found = true; break; }
                }
                if (!found) { error = "instance is not in the DataModel"; return false; }
                path = std::to_string(index) + (path.empty() ? "" : "/" + path);
                cur = parent;
            }
            out.type = ValueType::Ref;
            out.string_value = path;
            return true;
        }
        case ValueType::UDim: {
            auto* u = check_box(L, idx, UKind::UDim, "UDim");
            out.type = ValueType::UDim;
            out.complex_value = json_arr({Json(u->n[0]), Json(u->n[1])});
            return true;
        }
        case ValueType::UDim2: {
            auto* u = check_box(L, idx, UKind::UDim2, "UDim2");
            out.type = ValueType::UDim2;
            out.complex_value = json_arr({Json(u->n[0]), Json(u->n[1]), Json(u->n[2]), Json(u->n[3])});
            return true;
        }
        case ValueType::Tags: {
            if (!lua_istable(L, idx)) { error = "expected an array of strings for Tags"; return false; }
            out.type = ValueType::Tags;
            out.complex_value = Json::array();
            size_t len = lua_objlen(L, idx);
            for (size_t i = 1; i <= len; ++i) {
                lua_rawgeti(L, idx, static_cast<int>(i));
                out.complex_value.push(Json(lua_tostring(L, -1) ? lua_tostring(L, -1) : ""));
                lua_pop(L, 1);
            }
            return true;
        }
        default:
            error = std::string("setting properties of type ") + value_type_name(type) + " from Luau is not supported";
            return false;
    }
}

// Infer a variant for assigning to a property that has no existing value.
bool lua_to_variant_inferred(lua_State* L, int idx, Variant& out, std::string& error) {
    if (lua_isboolean(L, idx)) { out = Variant::make_bool(lua_toboolean(L, idx) != 0); return true; }
    if (lua_isnumber(L, idx)) { out = Variant::make_float64(lua_tonumber(L, idx)); return true; }
    if (lua_isstring(L, idx)) { out = Variant::make_string(lua_tostring(L, idx)); return true; }
    if (lua_isuserdata(L, idx)) {
        auto* u = static_cast<UData*>(lua_touserdata(L, idx));
        switch (u->kind) {
            case UKind::Vector3: return lua_to_variant(L, idx, ValueType::Vector3, out, error);
            case UKind::Color3: return lua_to_variant(L, idx, ValueType::Color3, out, error);
            case UKind::Vector2: return lua_to_variant(L, idx, ValueType::Vector2, out, error);
            case UKind::CFrame: return lua_to_variant(L, idx, ValueType::CFrame, out, error);
            case UKind::UDim: return lua_to_variant(L, idx, ValueType::UDim, out, error);
            case UKind::UDim2: return lua_to_variant(L, idx, ValueType::UDim2, out, error);
            case UKind::EnumItem: return lua_to_variant(L, idx, ValueType::EnumItem, out, error);
            default: break;
        }
    }
    error = "cannot infer a property type from that Luau value";
    return false;
}

std::string traceback_of(lua_State* co, const std::string& message) {
    luaL_traceback(co, co, message.c_str(), 0);
    std::string out = lua_tostring(co, -1) ? lua_tostring(co, -1) : message;
    lua_pop(co, 1);
    return out;
}

// --------------------------------------------------------------- instances --

void push_instance(lua_State* L, Instance* inst) {
    if (!inst) {
        lua_pushnil(L);
        return;
    }
    lua_getfield(L, LUA_REGISTRYINDEX, "blox_instcache");
    lua_pushlightuserdata(L, inst);
    lua_rawget(L, -2);
    if (!lua_isnil(L, -1)) {
        lua_remove(L, -2);
        return;
    }
    lua_pop(L, 1);
    UData* u = new_box(L, UKind::Instance, "BloxInstance");
    u->inst = inst;
    lua_pushlightuserdata(L, inst); // [cache, userdata, key]
    lua_pushvalue(L, -2);           // [cache, userdata, key, userdata]
    lua_rawset(L, -4);              // cache[ptr] = userdata
    lua_remove(L, -2);              // drop cache table, keep userdata
}

Instance* check_instance(lua_State* L, int idx) {
    void* mem = lua_touserdata(L, idx);
    if (!mem || static_cast<UData*>(mem)->kind != UKind::Instance)
        luaL_error(L, "expected Instance");
    return static_cast<UData*>(mem)->inst;
}

bool in_game(Runtime* rt, const Instance* inst) {
    const Instance* root = &rt->model().root();
    for (const Instance* cur = inst; cur; cur = cur->parent) {
        if (cur == root) return true;
    }
    return false;
}

void check_alive(Runtime* rt, Instance* inst) {
    if (rt->impl_->destroyed.count(inst))
        luaL_error(rt->state(), "attempt to use a destroyed instance '%s'", inst->name.c_str());
}

// Removes `inst` from wherever it is owned and returns sole ownership.
std::unique_ptr<Instance> extract_owned(Runtime* rt, Instance* inst) {
    if (inst->parent) {
        auto& siblings = inst->parent->children;
        for (auto it = siblings.begin(); it != siblings.end(); ++it) {
            if (it->get() == inst) {
                std::unique_ptr<Instance> owned = std::move(*it);
                siblings.erase(it);
                inst->parent = nullptr;
                return owned;
            }
        }
    }
    auto& detached = rt->impl_->detached;
    for (auto it = detached.begin(); it != detached.end(); ++it) {
        if (it->get() == inst) {
            std::unique_ptr<Instance> owned = std::move(*it);
            detached.erase(it);
            return owned;
        }
    }
    return nullptr;
}

void fire_instance_signal(Runtime* rt, lua_State* L, Instance* inst, const std::string& key,
                          const std::vector<LuaArg>& args) {
    if (SignalImpl* sig = rt->impl_->find_instance_signal(inst, key))
        fire_signal(rt, L, sig, args);
}

void fire_changed(Runtime* rt, lua_State* L, Instance* inst, const std::string& property) {
    fire_instance_signal(rt, L, inst, "Changed", {arg_string(property)});
    fire_instance_signal(rt, L, inst, "Changed:" + property, {arg_string(property)});
}

// Mint a fresh instance with engine reflection defaults. Created under a
// temporary staging folder (deleted immediately) so defaults apply.
Instance* mint_instance(Runtime* rt, const std::string& class_name, const std::string& name) {
    DataModel& model = rt->model();
    Instance& staging = model.add_instance(model.root(), "Folder", "__blox_staging");
    Instance& inst = model.add_instance(staging, class_name, name.empty() ? class_name : name);
    auto owned = extract_owned(rt, &inst);
    model.delete_instance(staging);
    Instance* raw = owned.get();
    rt->impl_->detached.push_back(std::move(owned));
    return raw;
}

void adopt_into(Runtime* rt, Instance* inst, Instance* new_parent) {
    auto owned = extract_owned(rt, inst);
    if (!owned) return; // already orphaned (e.g. destroyed)
    inst->parent = new_parent;
    new_parent->children.push_back(std::move(owned));
}

int inst_index(lua_State* L) {
    Runtime* rt = rt_of(L);
    auto* u = static_cast<UData*>(lua_touserdata(L, 1));
    Instance* inst = u->inst;
    const char* key_c = luaL_checkstring(L, 2);
    std::string key = key_c;

    if (key == "ClassName") { lua_pushstring(L, inst->class_name.c_str()); return 1; }
    if (key == "Name") { lua_pushstring(L, inst->name.c_str()); return 1; }
    if (key == "Parent") { push_instance(L, inst->parent); return 1; }
    check_alive(rt, inst);

    // DataModel metadata: absent values report the offline-unpublished
    // defaults (Roblox reports PlaceId 0 for unpublished local places).
    if (inst == &rt->model().root() &&
        (key == "PlaceId" || key == "GameId" || key == "PlaceVersion" ||
         key == "JobId" || key == "CreatorId" || key == "CreatorType")) {
        auto it = inst->properties.find(key);
        if (it != inst->properties.end()) {
            variant_to_lua(L, rt, it->second, key);
        } else if (key == "GameId" || key == "JobId") {
            lua_pushstring(L, "");
        } else if (key == "CreatorType") {
            push_enum_item(L, "CreatorType", "User", 0);
        } else {
            lua_pushinteger(L, 0);
        }
        return 1;
    }

    if (inst->class_name == "Players" && key == "LocalPlayer") {
        if (rt->impl_->net_context == Runtime::NetContext::Client)
            push_instance(L, rt->impl_->local_player);
        else
            lua_pushnil(L);
        return 1;
    }
    if (inst->class_name == "Player" && key == "Character") {
        if (rt->impl_->character_resolver) {
            push_instance(L, rt->impl_->character_resolver(inst));
            return 1;
        }
        // Client fallback: the replicated character model in Workspace
        // carries the player's name.
        Instance* workspace = nullptr;
        for (const auto& child : rt->model().root().children)
            if (child->class_name == "Workspace") { workspace = child.get(); break; }
        Instance* found = nullptr;
        if (workspace) {
            for (const auto& child : workspace->children)
                if (child->name == inst->name && child->class_name == "Model") { found = child.get(); break; }
        }
        push_instance(L, found);
        return 1;
    }
    if (inst->class_name == "RemoteFunction" &&
        (key == "OnServerInvoke" || key == "OnClientInvoke")) {
        auto it = rt->impl_->remote_invoke_handlers.find(inst);
        if (it != rt->impl_->remote_invoke_handlers.end()) {
            lua_rawgeti(L, LUA_REGISTRYINDEX, it->second);
        } else {
            lua_pushnil(L);
        }
        return 1;
    }

    // Service behaviours keyed by class.
    if (inst->class_name == "RunService" &&
        (key == "Heartbeat" || key == "Stepped" || key == "RenderStepped")) {
        push_signal(L, rt->impl_->instance_signal(rt, inst, key == "RenderStepped" ? "Stepped" : key));
        return 1;
    }
    if (inst->class_name == "Players" && (key == "PlayerAdded" || key == "PlayerRemoving")) {
        push_signal(L, rt->impl_->instance_signal(rt, inst, key));
        return 1;
    }
    if (inst->class_name == "UserInputService" && (key == "InputBegan" || key == "InputEnded")) {
        push_signal(L, rt->impl_->instance_signal(rt, inst, key));
        return 1;
    }

    // Methods.
    lua_getfield(L, LUA_REGISTRYINDEX, "blox_methods");
    lua_pushstring(L, key.c_str());
    lua_rawget(L, -2);
    if (!lua_isnil(L, -1)) { lua_remove(L, -2); return 1; }
    lua_pop(L, 2);

    // Signals.
    if (key == "Changed" || key == "ChildAdded" || key == "ChildRemoved" || key == "AncestryChanged" ||
        key == "Destroying" || key == "Died" || key == "OnServerEvent" ||
        key == "OnClientEvent" || key == "CharacterAdded" || key == "CharacterRemoving" ||
        key == "Touched" || key == "TouchEnded" || key == "StateChanged" ||
        // GUI events (Phase 22), fired by Runtime::gui_click/gui_hover/
        // gui_type_text against the runtime model's own layout.
        key == "MouseButton1Click" || key == "MouseButton2Click" || key == "Activated" ||
        key == "MouseEnter" || key == "MouseLeave" || key == "Focused" || key == "FocusLost") {
        push_signal(L, rt->impl_->instance_signal(rt, inst, key));
        return 1;
    }

    // Properties.
    auto it = inst->properties.find(key);
    if (it != inst->properties.end()) {
        variant_to_lua(L, rt, it->second, key);
        return 1;
    }
    // Derived Position mirrors CFrame translation for BaseParts.
    if (key == "Position") {
        auto cf = inst->properties.find("CFrame");
        if (cf != inst->properties.end() && cf->second.type == ValueType::CFrame) {
            push_vector3(L, cf->second.position[0], cf->second.position[1], cf->second.position[2]);
            return 1;
        }
    }

    // Child instances by name.
    for (const auto& child : inst->children) {
        if (child->name == key) {
            push_instance(L, child.get());
            return 1;
        }
    }
    lua_pushnil(L);
    return 1;
}

int inst_newindex(lua_State* L) {
    Runtime* rt = rt_of(L);
    DataModel& model = rt->model();
    auto* u = static_cast<UData*>(lua_touserdata(L, 1));
    Instance* inst = u->inst;
    std::string key = luaL_checkstring(L, 2);
    check_alive(rt, inst);

    // DataModel metadata is read-only from Luau (as in Roblox).
    if (inst == &model.root() &&
        (key == "PlaceId" || key == "GameId" || key == "PlaceVersion" ||
         key == "JobId" || key == "CreatorId" || key == "CreatorType"))
        luaL_error(L, "%s is a read-only property", key.c_str());

    if (key == "Parent") {
        Instance* old_parent = inst->parent;
        if (lua_isnil(L, 3)) {
            if (old_parent) {
                fire_instance_signal(rt, L, old_parent, "ChildRemoved", {arg_instance(inst)});
                auto owned = extract_owned(rt, inst);
                if (owned) rt->impl_->detached.push_back(std::move(owned));
            }
            fire_instance_signal(rt, L, inst, "AncestryChanged", {});
            return 0;
        }
        Instance* new_parent = check_instance(L, 3);
        if (new_parent == inst) luaL_error(L, "attempt to set Parent to itself");
        for (Instance* cur = new_parent; cur; cur = cur->parent) {
            if (cur == inst) luaL_error(L, "attempt to set Parent to a descendant (cycle)");
        }
        if (old_parent)
            fire_instance_signal(rt, L, old_parent, "ChildRemoved", {arg_instance(inst)});
        if (!in_game(rt, inst) || !in_game(rt, new_parent)) {
            adopt_into(rt, inst, new_parent);
        } else {
            std::string error;
            if (!model.reparent(*inst, *new_parent, error)) luaL_error(L, "%s", error.c_str());
        }
        fire_instance_signal(rt, L, new_parent, "ChildAdded", {arg_instance(inst)});
        fire_instance_signal(rt, L, inst, "AncestryChanged", {arg_instance(new_parent)});
        fire_changed(rt, L, inst, "Parent");
        return 0;
    }

    if (inst->class_name == "RemoteFunction" &&
        (key == "OnServerInvoke" || key == "OnClientInvoke")) {
        if (lua_isnil(L, 3)) {
            rt->impl_->remote_invoke_handlers.erase(inst);
        } else {
            luaL_checktype(L, 3, LUA_TFUNCTION);
            lua_pushvalue(L, 3);
            int ref = lua_ref(L, -1);
            lua_pop(L, 1);
            rt->impl_->remote_invoke_handlers[inst] = ref;
        }
        return 0;
    }

    std::string error;
    Variant value;
    auto existing = inst->properties.find(key);
    if (key == "Name") {
        const char* name = luaL_checkstring(L, 3);
        if (!model.set_property(*inst, "Name", Variant::make_string(name), error))
            luaL_error(L, "%s", error.c_str());
        fire_changed(rt, L, inst, "Name");
        return 0;
    }
    if (existing != inst->properties.end()) {
        if (!lua_to_variant(L, 3, existing->second.type, value, error))
            luaL_error(L, "bad value for %s.%s: %s", inst->class_name.c_str(), key.c_str(), error.c_str());
    } else if (is_known_class(inst->class_name)) {
        // Known class without this property: Roblox would error. Our
        // reflection table is partial, so only reject when the property is
        // entirely inferable — otherwise store verbatim like the engine.
        if (!lua_to_variant_inferred(L, 3, value, error))
            luaL_error(L, "%s is not a valid member of %s", key.c_str(), inst->class_name.c_str());
    } else {
        if (!lua_to_variant_inferred(L, 3, value, error))
            luaL_error(L, "%s", error.c_str());
    }
    if (!model.set_property(*inst, key, value, error))
        luaL_error(L, "%s", error.c_str());
    fire_changed(rt, L, inst, key);
    return 0;
}

int inst_eq(lua_State* L) {
    auto* a = static_cast<UData*>(lua_touserdata(L, 1));
    auto* b = static_cast<UData*>(lua_touserdata(L, 2));
    lua_pushboolean(L, (a && b && a->inst == b->inst) ? 1 : 0);
    return 1;
}

int inst_tostring(lua_State* L) {
    auto* u = static_cast<UData*>(lua_touserdata(L, 1));
    lua_pushstring(L, u && u->inst ? u->inst->name.c_str() : "Instance");
    return 1;
}

// --------------------------------------------------------- instance methods --

int m_find_first_child(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* inst = check_instance(L, 1);
    check_alive(rt, inst);
    const char* name = luaL_checkstring(L, 2);
    bool recursive = lua_toboolean(L, 3) != 0;
    std::vector<Instance*> stack{inst};
    bool first = true;
    while (!stack.empty()) {
        Instance* cur = stack.back();
        stack.pop_back();
        if (!first && cur->name == name) { push_instance(L, cur); return 1; }
        first = false;
        if (!recursive && cur != inst) continue;
        for (auto it = cur->children.rbegin(); it != cur->children.rend(); ++it) stack.push_back(it->get());
    }
    lua_pushnil(L);
    return 1;
}

int m_find_first_child_of_class(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* inst = check_instance(L, 1);
    check_alive(rt, inst);
    const char* cls = luaL_checkstring(L, 2);
    bool recursive = lua_toboolean(L, 3) != 0;
    std::vector<Instance*> stack{inst};
    bool first = true;
    while (!stack.empty()) {
        Instance* cur = stack.back();
        stack.pop_back();
        if (!first && cur->class_name == cls) { push_instance(L, cur); return 1; }
        first = false;
        if (!recursive && cur != inst) continue;
        for (auto it = cur->children.rbegin(); it != cur->children.rend(); ++it) stack.push_back(it->get());
    }
    lua_pushnil(L);
    return 1;
}

int m_get_children(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* inst = check_instance(L, 1);
    check_alive(rt, inst);
    lua_newtable(L);
    int i = 1;
    for (const auto& child : inst->children) {
        push_instance(L, child.get());
        lua_rawseti(L, -2, i++);
    }
    return 1;
}

int m_get_descendants(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* inst = check_instance(L, 1);
    check_alive(rt, inst);
    lua_newtable(L);
    int i = 1;
    std::vector<Instance*> stack;
    for (auto it = inst->children.rbegin(); it != inst->children.rend(); ++it) stack.push_back(it->get());
    while (!stack.empty()) {
        Instance* cur = stack.back();
        stack.pop_back();
        push_instance(L, cur);
        lua_rawseti(L, -2, i++);
        for (auto it = cur->children.rbegin(); it != cur->children.rend(); ++it) stack.push_back(it->get());
    }
    return 1;
}

bool class_is_a(const std::string& cls, const std::string& query) {
    if (cls == query || query == "Instance") return true;
    static const std::map<std::string, std::vector<std::string>> kAncestors = {
        {"Part", {"BasePart"}}, {"MeshPart", {"BasePart"}}, {"WedgePart", {"BasePart"}},
        {"CornerWedgePart", {"BasePart"}}, {"SpawnLocation", {"BasePart"}},
        {"Frame", {"GuiObject"}}, {"CanvasGroup", {"GuiObject"}}, {"TextLabel", {"GuiObject"}},
        {"TextButton", {"GuiObject", "GuiButton"}}, {"TextBox", {"GuiObject"}},
        {"ImageLabel", {"GuiObject"}}, {"ImageButton", {"GuiObject", "GuiButton"}},
        {"ScrollingFrame", {"GuiObject"}}, {"ScreenGui", {"LayerCollector"}},
        {"LocalScript", {"BaseScript"}}, {"Script", {"BaseScript"}},
    };
    auto it = kAncestors.find(cls);
    if (it == kAncestors.end()) return false;
    return std::find(it->second.begin(), it->second.end(), query) != it->second.end();
}

int m_is_a(lua_State* L) {
    Instance* inst = check_instance(L, 1);
    const char* query = luaL_checkstring(L, 2);
    lua_pushboolean(L, class_is_a(inst->class_name, query) ? 1 : 0);
    return 1;
}

int m_clone(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* inst = check_instance(L, 1);
    check_alive(rt, inst);
    Json json = DataModel::instance_to_json(*inst);
    uint64_t next = rt->impl_->next_referent;
    std::string error;
    auto copy = DataModel::instance_from_json(json, next, error);
    rt->impl_->next_referent = next;
    if (!copy) luaL_error(L, "Clone failed: %s", error.c_str());
    copy->parent = nullptr;
    Instance* raw = copy.get();
    rt->impl_->detached.push_back(std::move(copy));
    push_instance(L, raw);
    return 1;
}

int m_destroy(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* inst = check_instance(L, 1);
    if (inst == &rt->model().root()) luaL_error(L, "cannot Destroy the DataModel");
    if (rt->impl_->destroyed.count(inst)) return 0;
    Instance* old_parent = inst->parent;
    fire_instance_signal(rt, L, inst, "Destroying", {});
    if (old_parent) fire_instance_signal(rt, L, old_parent, "ChildRemoved", {arg_instance(inst)});
    auto owned = extract_owned(rt, inst);
    rt->impl_->destroyed.insert(inst);
    if (owned) rt->impl_->graveyard.push_back(std::move(owned));
    return 0;
}

int m_clear_children(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* inst = check_instance(L, 1);
    check_alive(rt, inst);
    while (!inst->children.empty()) {
        Instance* child = inst->children.front().get();
        fire_instance_signal(rt, L, inst, "ChildRemoved", {arg_instance(child)});
        auto owned = extract_owned(rt, child);
        rt->impl_->destroyed.insert(child);
        if (owned) rt->impl_->graveyard.push_back(std::move(owned));
    }
    return 0;
}

int m_get_full_name(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* inst = check_instance(L, 1);
    lua_pushstring(L, rt->model().path_of(*inst).c_str());
    return 1;
}

int m_get_service(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* inst = check_instance(L, 1);
    DataModel& model = rt->model();
    if (inst != &model.root())
        luaL_error(L, "GetService is only available on the DataModel (game)");
    const char* service = luaL_checkstring(L, 2);
    for (const auto& child : model.root().children) {
        if (child->class_name == service) { push_instance(L, child.get()); return 1; }
    }
    static const char* kServices[] = {"Workspace", "Lighting", "ReplicatedStorage", "ServerStorage",
        "ServerScriptService", "StarterGui", "StarterPlayer", "StarterPack", "Teams", "SoundService",
        "RunService", "Players", "TweenService", "CollectionService", "UserInputService", "TextService", "PhysicsService", "ContextActionService", "PathfindingService"};
    for (const char* known : kServices) {
        if (std::string(service) == known) {
            Instance& created = model.add_instance(model.root(), service, service);
            push_instance(L, &created);
            return 1;
        }
    }
    luaL_error(L, "service '%s' not found", service);
    return 0;
}

Variant lua_value_to_attribute(lua_State* L, int idx, bool& ok) {
    ok = true;
    std::string error;
    Variant out;
    if (!lua_to_variant_inferred(L, idx, out, error)) ok = false;
    return out;
}

int m_get_attribute(lua_State* L) {
    Instance* inst = check_instance(L, 1);
    const char* name = luaL_checkstring(L, 2);
    auto it = inst->properties.find("Attributes");
    if (it != inst->properties.end() && it->second.complex_value.is_object()) {
        if (const Json* entry = it->second.complex_value.find(name)) {
            Variant v;
            std::string error;
            Json wrapped = Json::object();
            wrapped.set("type", Json(entry->find("type") ? entry->find("type")->as_string() : "String"));
            if (entry->find("value")) {
                Json val = *entry->find("value");
                wrapped.set("value", std::move(val));
            }
            if (Variant::from_json(wrapped, v, error)) {
                Runtime* rt = rt_of(L);
                variant_to_lua(L, rt, v, name);
                return 1;
            }
        }
    }
    lua_pushnil(L);
    return 1;
}

int m_set_attribute(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* inst = check_instance(L, 1);
    check_alive(rt, inst);
    const char* name = luaL_checkstring(L, 2);
    Variant attrs;
    auto existing = inst->properties.find("Attributes");
    if (existing != inst->properties.end()) attrs = existing->second;
    attrs.type = ValueType::Attributes;
    if (!attrs.complex_value.is_object()) attrs.complex_value = Json::object();
    if (lua_isnil(L, 3)) {
        // Remove: rebuild object without the key.
        Json rebuilt = Json::object();
        for (const auto& [k, v] : attrs.complex_value.as_object()) {
            if (k != name) { Json copy = v; rebuilt.set(k, std::move(copy)); }
        }
        attrs.complex_value = rebuilt;
    } else {
        bool ok = false;
        Variant value = lua_value_to_attribute(L, 3, ok);
        if (!ok) luaL_error(L, "unsupported attribute value type");
        attrs.complex_value.set(name, value.to_json());
    }
    std::string error;
    if (!rt->model().set_property(*inst, "Attributes", attrs, error))
        luaL_error(L, "%s", error.c_str());
    fire_changed(rt, L, inst, "Attributes");
    return 0;
}

int m_get_property_changed_signal(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* inst = check_instance(L, 1);
    const char* prop = luaL_checkstring(L, 2);
    push_signal(L, rt->impl_->instance_signal(rt, inst, std::string("Changed:") + prop));
    return 1;
}

// Tags (shared by Instance methods and CollectionService).
std::vector<std::string> tags_of(Instance* inst) {
    std::vector<std::string> tags;
    auto it = inst->properties.find("Tags");
    if (it != inst->properties.end() && it->second.complex_value.is_array()) {
        for (const Json& t : it->second.complex_value.as_array()) tags.push_back(t.as_string());
    }
    return tags;
}

void set_tags(Runtime* rt, lua_State* L, Instance* inst, const std::vector<std::string>& tags) {
    Variant v;
    v.type = ValueType::Tags;
    v.complex_value = Json::array();
    for (const std::string& t : tags) v.complex_value.push(Json(t));
    std::string error;
    rt->model().set_property(*inst, "Tags", v, error);
    fire_changed(rt, L, inst, "Tags");
}

int m_add_tag(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* inst = check_instance(L, 1);
    check_alive(rt, inst);
    const char* tag = luaL_checkstring(L, 2);
    auto tags = tags_of(inst);
    if (std::find(tags.begin(), tags.end(), tag) == tags.end()) {
        tags.emplace_back(tag);
        set_tags(rt, L, inst, tags);
        auto it = rt->impl_->tag_signals.find(std::string("added:") + tag);
        if (it != rt->impl_->tag_signals.end()) fire_signal(rt, L, it->second, {arg_instance(inst)});
    }
    return 0;
}

int m_remove_tag(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* inst = check_instance(L, 1);
    check_alive(rt, inst);
    const char* tag = luaL_checkstring(L, 2);
    auto tags = tags_of(inst);
    auto it = std::find(tags.begin(), tags.end(), tag);
    if (it != tags.end()) {
        tags.erase(it);
        set_tags(rt, L, inst, tags);
        auto sig = rt->impl_->tag_signals.find(std::string("removed:") + tag);
        if (sig != rt->impl_->tag_signals.end()) fire_signal(rt, L, sig->second, {arg_instance(inst)});
    }
    return 0;
}

int m_has_tag(lua_State* L) {
    Instance* inst = check_instance(L, 1);
    const char* tag = luaL_checkstring(L, 2);
    auto tags = tags_of(inst);
    lua_pushboolean(L, std::find(tags.begin(), tags.end(), tag) != tags.end() ? 1 : 0);
    return 1;
}

int m_get_tags(lua_State* L) {
    Instance* inst = check_instance(L, 1);
    lua_newtable(L);
    int i = 1;
    for (const std::string& t : tags_of(inst)) {
        lua_pushstring(L, t.c_str());
        lua_rawseti(L, -2, i++);
    }
    return 1;
}

// ------------------------------------------------------------------ signals --

int sig_connect_impl(lua_State* L, bool once) {
    Runtime* rt = rt_of(L);
    auto* u = check_box(L, 1, UKind::Signal, "RBXScriptSignal");
    luaL_checktype(L, 2, LUA_TFUNCTION);
    SignalImpl* sig = u->signal;
    int ref = lua_ref(L, -1); lua_pop(L, 1);
    int64_t id = sig->next_id++;
    sig->conns[id] = SignalImpl::Conn{ref, true, once};
    UData* c = new_box(L, UKind::Connection, "BloxConnection");
    c->signal = sig;
    c->conn_id = id;
    c->flag = true;
    c->n[0] = once ? 1 : 0;
    (void)rt;
    return 1;
}
int sig_connect(lua_State* L) { return sig_connect_impl(L, false); }
int sig_once(lua_State* L) { return sig_connect_impl(L, true); }

int sig_wait(lua_State* L) {
    Runtime* rt = rt_of(L);
    auto* u = check_box(L, 1, UKind::Signal, "RBXScriptSignal");
    for (auto& t : rt->impl_->threads) {
        if (t->co == L && !t->dead) {
            t->wait = Thread::Wait::Signal;
            t->waiting_on = u->signal;
            u->signal->waiters.push_back(t.get());
            return lua_yield(L, 0);
        }
    }
    luaL_error(L, "Signal:Wait() must be called from a task thread");
    return 0;
}

int conn_index(lua_State* L) {
    auto* u = check_box(L, 1, UKind::Connection, "RBXScriptConnection");
    std::string key = luaL_checkstring(L, 2);
    if (key == "Connected") {
        bool connected = u->flag;
        if (u->signal) {
            auto it = u->signal->conns.find(u->conn_id);
            connected = it != u->signal->conns.end() && it->second.connected;
        }
        lua_pushboolean(L, connected ? 1 : 0);
        return 1;
    }
    lua_getfield(L, LUA_REGISTRYINDEX, "blox_connection_methods");
    lua_pushstring(L, key.c_str());
    lua_rawget(L, -2);
    lua_remove(L, -2);
    return 1;
}

int conn_disconnect(lua_State* L) {
    Runtime* rt = rt_of(L);
    auto* u = check_box(L, 1, UKind::Connection, "RBXScriptConnection");
    u->flag = false;
    if (u->signal) {
        auto it = u->signal->conns.find(u->conn_id);
        if (it != u->signal->conns.end() && it->second.connected) {
            it->second.connected = false;
            lua_unref(rt->state(), it->second.fn_ref);
        }
    }
    return 0;
}

// ------------------------------------------------------------------- tweens --

double apply_easing(const std::string& style, double t) {
    if (style == "Sine") return 0.5 - 0.5 * std::cos(3.14159265358979 * t);
    if (style == "Quad") return t * t;
    if (style == "Cubic") return t * t * t;
    if (style == "Quart") { double t2 = t * t; return t2 * t2; }
    if (style == "Back") { const double c = 1.70158; return (c + 1) * t * t * t - c * t * t; }
    return t; // Linear and everything else (documented approximation)
}

Variant interpolate_variant(const Variant& from, const Variant& to, double a) {
    Variant out = to;
    auto lerp = [](double x, double y, double t) { return x + (y - x) * t; };
    switch (to.type) {
        case ValueType::Float32: case ValueType::Float64:
            out.float_value = lerp(from.float_value, to.float_value, a); break;
        case ValueType::Int32: case ValueType::Int64:
            out.int_value = static_cast<int64_t>(lerp(static_cast<double>(from.int_value),
                                                      static_cast<double>(to.int_value), a)); break;
        case ValueType::Vector3: case ValueType::Color3:
            for (int i = 0; i < 3; ++i) out.vec3[i] = lerp(from.vec3[i], to.vec3[i], a);
            break;
        case ValueType::Vector2: {
            if (from.complex_value.is_array() && to.complex_value.is_array()) {
                out.complex_value = json_arr({
                    Json(lerp(from.complex_value.as_array()[0].as_double(), to.complex_value.as_array()[0].as_double(), a)),
                    Json(lerp(from.complex_value.as_array()[1].as_double(), to.complex_value.as_array()[1].as_double(), a))});
            }
            break;
        }
        case ValueType::CFrame:
            for (int i = 0; i < 3; ++i) out.position[i] = lerp(from.position[i], to.position[i], a);
            break; // rotation snaps at completion (documented approximation)
        default: break;
    }
    return out;
}

int tween_play(lua_State* L) {
    auto* u = check_box(L, 1, UKind::Tween, "Tween");
    auto* tween = static_cast<TweenState*>(u->extra);
    tween->starts.clear();
    for (auto& [prop, goal] : tween->goals) {
        auto it = tween->inst->properties.find(prop);
        tween->starts.push_back({prop, it != tween->inst->properties.end() ? it->second : goal});
    }
    tween->elapsed = 0;
    tween->playing = true;
    return 0;
}
int tween_pause(lua_State* L) {
    auto* u = check_box(L, 1, UKind::Tween, "Tween");
    static_cast<TweenState*>(u->extra)->playing = false;
    return 0;
}
int tween_cancel(lua_State* L) {
    Runtime* rt = rt_of(L);
    auto* u = check_box(L, 1, UKind::Tween, "Tween");
    auto* tween = static_cast<TweenState*>(u->extra);
    tween->playing = false;
    if (tween->completed) fire_signal(rt, L, tween->completed, {arg_string("Cancelled")});
    return 0;
}
int tween_index(lua_State* L) {
    Runtime* rt = rt_of(L);
    auto* u = check_box(L, 1, UKind::Tween, "Tween");
    auto* tween = static_cast<TweenState*>(u->extra);
    std::string key = luaL_checkstring(L, 2);
    if (key == "Completed") { push_signal(L, tween->completed); return 1; }
    if (key == "PlaybackState") {
        lua_pushstring(L, tween->playing ? "Playing" : "Paused");
        return 1;
    }
    lua_getfield(L, LUA_REGISTRYINDEX, "blox_tween_methods");
    lua_pushstring(L, key.c_str());
    lua_rawget(L, -2);
    lua_remove(L, -2);
    (void)rt;
    return 1;
}

// ------------------------------------------------------- service behaviours --

int m_tween_create(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* target = check_instance(L, 2);
    check_alive(rt, target);
    double duration = 1.0;
    std::string easing = "Linear";
    if (lua_istable(L, 3)) {
        lua_getfield(L, 3, "Time");
        if (lua_isnumber(L, -1)) duration = lua_tonumber(L, -1);
        lua_pop(L, 1);
        lua_getfield(L, 3, "EasingStyle");
        if (lua_isuserdata(L, -1)) easing = static_cast<UData*>(lua_touserdata(L, -1))->s2;
        else if (lua_isstring(L, -1)) easing = lua_tostring(L, -1);
        lua_pop(L, 1);
    }
    auto tween = std::make_unique<TweenState>();
    tween->inst = target;
    tween->duration = std::max(duration, 0.0);
    tween->completed = rt->impl_->new_signal(rt);
    if (lua_istable(L, 4)) {
        lua_pushnil(L);
        while (lua_next(L, 4) != 0) {
            const char* prop = lua_tostring(L, -2);
            auto existing = target->properties.find(prop ? prop : "");
            Variant goal;
            std::string error;
            bool ok = false;
            if (existing != target->properties.end())
                ok = lua_to_variant(L, -1, existing->second.type, goal, error);
            else
                ok = lua_to_variant_inferred(L, -1, goal, error);
            if (ok && prop) tween->goals.push_back({prop, goal});
            lua_pop(L, 1);
        }
    }
    (void)easing; // stored below via s1 on the userdata is not needed; keep in TweenState:
    TweenState* raw = tween.get();
    rt->impl_->tweens.push_back(std::move(tween));
    UData* u = new_box(L, UKind::Tween, "BloxTween");
    u->extra = raw;
    u->s1 = easing;
    return 1;
}

int m_collection_add_tag(lua_State* L) {
    // (service, instance, tag) — delegate to the instance method shape.
    lua_remove(L, 1);
    return m_add_tag(L);
}
int m_collection_remove_tag(lua_State* L) {
    lua_remove(L, 1);
    return m_remove_tag(L);
}
int m_collection_has_tag(lua_State* L) {
    lua_remove(L, 1);
    return m_has_tag(L);
}
int m_collection_get_tagged(lua_State* L) {
    Runtime* rt = rt_of(L);
    const char* tag = luaL_checkstring(L, 2);
    lua_newtable(L);
    int i = 1;
    std::vector<Instance*> stack{&rt->model().root()};
    while (!stack.empty()) {
        Instance* cur = stack.back();
        stack.pop_back();
        auto tags = tags_of(cur);
        if (std::find(tags.begin(), tags.end(), tag) != tags.end()) {
            push_instance(L, cur);
            lua_rawseti(L, -2, i++);
        }
        for (auto it = cur->children.rbegin(); it != cur->children.rend(); ++it) stack.push_back(it->get());
    }
    return 1;
}
int m_collection_tag_signal(lua_State* L) {
    Runtime* rt = rt_of(L);
    const char* tag = luaL_checkstring(L, 2);
    // The signal name encodes add/remove via the method used; both map here
    // with the caller's requested kind stored in the key prefix.
    std::string key = std::string(lua_tostring(L, lua_upvalueindex(1))) + tag;
    auto [it, inserted] = rt->impl_->tag_signals.try_emplace(key, nullptr);
    if (inserted) it->second = rt->impl_->new_signal(rt);
    push_signal(L, it->second);
    return 1;
}

int m_players_get_players(lua_State* L) {
    lua_newtable(L);
    return 1;
}
int m_runservice_is_running(lua_State* L) { lua_pushboolean(L, 1); return 1; }
int m_runservice_is_server(lua_State* L) { lua_pushboolean(L, 1); return 1; }
int m_runservice_is_client(lua_State* L) { lua_pushboolean(L, 0); return 1; }
int m_unsupported(lua_State* L) {
    luaL_error(L, "this API is not supported by the BloxCLI runtime yet");
    return 0;
}

int m_uis_is_key_down(lua_State* L) {
    Runtime* rt = rt_of(L);
    std::string key;
    if (lua_isstring(L, 2)) key = lua_tostring(L, 2);
    else if (lua_isuserdata(L, 2)) key = static_cast<UData*>(lua_touserdata(L, 2))->s2;
    auto it = rt->impl_->key_states.find(key);
    lua_pushboolean(L, (it != rt->impl_->key_states.end() && it->second) ? 1 : 0);
    return 1;
}

// --------------------------------------------------------------------- task --

int task_wait(lua_State* L) {
    double delay = luaL_optnumber(L, 1, 0.0);
    lua_pushnumber(L, delay);
    return lua_yield(L, 1);
}

int task_spawn(lua_State* L) {
    Runtime* rt = rt_of(L);
    luaL_checktype(L, 1, LUA_TFUNCTION);
    int nargs = lua_gettop(L) - 1;
    Thread* t = make_thread(rt, L, rt->impl_->current_owner);
    lua_pushvalue(L, 1);
    lua_xmove(L, t->co, 1);
    for (int i = 0; i < nargs; ++i) {
        lua_pushvalue(L, 2 + i);
        lua_xmove(L, t->co, 1);
    }
    t->pending_args = nargs;
    t->wake = rt->time();
    return 0;
}
int task_defer(lua_State* L) {
    Runtime* rt = rt_of(L);
    int result = task_spawn(L);
    Thread* t = rt->impl_->threads.back().get();
    t->ready_step = rt->impl_->step_index + 1;
    return result;
}
int task_delay(lua_State* L) {
    Runtime* rt = rt_of(L);
    double delay = luaL_checknumber(L, 1);
    lua_remove(L, 1);
    int result = task_spawn(L);
    rt->impl_->threads.back()->wake = rt->time() + delay;
    return result;
}
int task_clock(lua_State* L) {
    lua_pushnumber(L, rt_of(L)->time());
    return 1;
}

int fn_instance_new(lua_State* L) {
    Runtime* rt = rt_of(L);
    const char* cls = luaL_checkstring(L, 1);
    Instance* inst = mint_instance(rt, cls, cls);
    if (lua_gettop(L) >= 2 && !lua_isnil(L, 2)) {
        Instance* parent = check_instance(L, 2);
        fire_instance_signal(rt, L, parent, "ChildAdded", {arg_instance(inst)});
        adopt_into(rt, inst, parent);
    }
    push_instance(L, inst);
    return 1;
}

int fn_require(lua_State* L) {
    Runtime* rt = rt_of(L);
    DataModel& model = rt->model();
    if (!lua_isuserdata(L, 1)) {
        luaL_error(L, "require by asset id is not available in the offline runtime; require a ModuleScript instance");
    }
    Instance* inst = check_instance(L, 1);
    if (inst->class_name != "ModuleScript")
        luaL_error(L, "require expects a ModuleScript, got %s", inst->class_name.c_str());
    lua_getfield(L, LUA_REGISTRYINDEX, "blox_modulecache");
    lua_pushlightuserdata(L, inst);
    lua_rawget(L, -2);
    if (!lua_isnil(L, -1)) { lua_remove(L, -2); return 1; }
    lua_pop(L, 2);
    if (rt->impl_->modules_loading.count(inst))
        luaL_error(L, "require recursion detected for ModuleScript '%s'", inst->name.c_str());
    auto source_it = inst->properties.find("Source");
    std::string source = (source_it != inst->properties.end() && source_it->second.type == ValueType::String)
                             ? source_it->second.string_value : "";
    rt->impl_->modules_loading.insert(inst);
    Thread* t = make_thread(rt, L, model.path_of(*inst));
    lua_State* co = t->co;
    push_instance(co, inst);
    lua_setglobal(co, "script");
    lua_CompileOptions opts{};
    opts.optimizationLevel = 1;
    opts.debugLevel = 2;
    size_t bc_size = 0;
    char* bc = luau_compile(source.data(), source.size(), &opts, &bc_size);
    std::string chunk = "=" + model.path_of(*inst);
    int load_status = luau_load(co, chunk.c_str(), bc, bc_size, 0);
    std::free(bc);
    if (load_status != 0) {
        std::string msg = lua_tostring(co, -1) ? lua_tostring(co, -1) : "compile error";
        rt->impl_->modules_loading.erase(inst);
        t->dead = true;
        luaL_error(L, "%s", msg.c_str());
    }
    // Run the module to completion synchronously (task.wait inside module
    // initialisation is compressed; documented).
    int status = LUA_OK;
    int guard = 0;
    do {
        status = lua_resume(co, L, 0);
    } while (status == LUA_YIELD && ++guard < 10000);
    rt->impl_->modules_loading.erase(inst);
    if (status != LUA_OK) {
        std::string msg = lua_tostring(co, -1) ? lua_tostring(co, -1) : "module error";
        std::string tb = traceback_of(co, msg);
        rt->report_error(msg, tb, model.path_of(*inst));
        t->dead = true;
        luaL_error(L, "%s", msg.c_str());
    }
    if (lua_gettop(co) != 1) {
        t->dead = true;
        luaL_error(L, "ModuleScript '%s' must return exactly one value", inst->name.c_str());
    }
    // Cache and move the result to the caller: stack is [arg] here.
    lua_xmove(co, L, 1); // [arg, result]
    lua_getfield(L, LUA_REGISTRYINDEX, "blox_modulecache");
    lua_pushlightuserdata(L, inst);
    lua_pushvalue(L, -3);
    lua_rawset(L, -3); // cache[inst] = result
    lua_pop(L, 1);     // drop cache table, leaving [arg, result]
    t->dead = true;
    lua_unref(L, t->co_ref);
    return 1;
}


// ------------------------------------------------------------------ remotes --

Instance* check_remote(lua_State* L, int idx, const char* klass) {
    Instance* inst = check_instance(L, idx);
    if (inst->class_name != klass)
        luaL_error(L, "this method is only available on %s instances", klass);
    return inst;
}

int m_remote_fire_server(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* inst = check_remote(L, 1, "RemoteEvent");
    if (rt->impl_->net_context != Runtime::NetContext::Client)
        luaL_error(L, "FireServer must be called from the client (LocalScript context)");
    if (!rt->impl_->remote_sink) luaL_error(L, "no server is connected");
    std::string error;
    Json args = lua_args_to_json(L, 2, error);
    if (!error.empty()) luaL_error(L, "%s", error.c_str());
    rt->impl_->remote_sink(Runtime::RemoteCall{inst, 0, rt->impl_->player_index, 0, args.dump()});
    return 0;
}

int m_remote_fire_client(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* inst = check_remote(L, 1, "RemoteEvent");
    if (rt->impl_->net_context == Runtime::NetContext::Client)
        luaL_error(L, "FireClient must be called from the server");
    if (!rt->impl_->remote_sink) luaL_error(L, "no clients are connected");
    std::string error;
    Json args = lua_args_to_json(L, 2, error);
    if (!error.empty()) luaL_error(L, "%s", error.c_str());
    rt->impl_->remote_sink(Runtime::RemoteCall{inst, 1, rt->impl_->player_index, 0, args.dump()});
    return 0;
}

int m_remote_fire_all_clients(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* inst = check_remote(L, 1, "RemoteEvent");
    if (rt->impl_->net_context == Runtime::NetContext::Client)
        luaL_error(L, "FireAllClients must be called from the server");
    if (!rt->impl_->remote_sink) luaL_error(L, "no clients are connected");
    std::string error;
    Json args = lua_args_to_json(L, 2, error);
    if (!error.empty()) luaL_error(L, "%s", error.c_str());
    rt->impl_->remote_sink(Runtime::RemoteCall{inst, 2, rt->impl_->player_index, 0, args.dump()});
    return 0;
}

int m_remote_invoke_impl(lua_State* L, bool to_server) {
    Runtime* rt = rt_of(L);
    Instance* inst = check_remote(L, 1, "RemoteFunction");
    if (to_server && rt->impl_->net_context != Runtime::NetContext::Client)
        luaL_error(L, "InvokeServer must be called from the client");
    if (!to_server && rt->impl_->net_context == Runtime::NetContext::Client)
        luaL_error(L, "InvokeClient must be called from the server");
    if (!rt->impl_->remote_sink) luaL_error(L, "no remote peer is connected");
    Thread* self = nullptr;
    for (auto& t : rt->impl_->threads) {
        if (t->co == L && !t->dead) self = t.get();
    }
    if (!self) luaL_error(L, "InvokeServer/InvokeClient must be called from a task thread");
    std::string error;
    Json args = lua_args_to_json(L, 2, error);
    if (!error.empty()) luaL_error(L, "%s", error.c_str());
    int id = rt->impl_->next_invoke_id++;
    rt->impl_->pending_invokes[id] = self;
    self->wait = Thread::Wait::Signal; // parked until complete_invoke
    rt->impl_->remote_sink(Runtime::RemoteCall{inst, to_server ? 3 : 4, rt->impl_->player_index, id, args.dump()});
    return lua_yield(L, 0);
}
int m_remote_invoke_server(lua_State* L) { return m_remote_invoke_impl(L, true); }
int m_remote_invoke_client(lua_State* L) { return m_remote_invoke_impl(L, false); }


int m_workspace_get_parts_in_box(lua_State* L) {
    Runtime* rt = rt_of(L);
    if (!rt->impl_->physics)
        luaL_error(L, "Workspace:GetPartBoundsInBox is only available during physics-enabled play/net sessions");
    UData* cf = check_box(L, 2, UKind::CFrame, "CFrame");
    UData* size = check_box(L, 3, UKind::Vector3, "Vector3");
    auto hits = rt->impl_->physics->parts_in_box(
        {cf->n[0], cf->n[1], cf->n[2]}, {size->n[0], size->n[1], size->n[2]});
    lua_newtable(L);
    int i = 1;
    for (Instance* inst : hits) { push_instance(L, inst); lua_rawseti(L, -2, i++); }
    return 1;
}

int m_workspace_get_parts_in_radius(lua_State* L) {
    Runtime* rt = rt_of(L);
    if (!rt->impl_->physics)
        luaL_error(L, "Workspace:GetPartBoundsInRadius is only available during physics-enabled play/net sessions");
    UData* pos = check_box(L, 2, UKind::Vector3, "Vector3");
    double radius = luaL_checknumber(L, 3);
    auto hits = rt->impl_->physics->parts_in_radius({pos->n[0], pos->n[1], pos->n[2]}, radius);
    lua_newtable(L);
    int i = 1;
    for (Instance* inst : hits) { push_instance(L, inst); lua_rawseti(L, -2, i++); }
    return 1;
}

int m_workspace_raycast(lua_State* L) {
    Runtime* rt = rt_of(L);
    if (!rt->impl_->physics)
        luaL_error(L, "Workspace:Raycast is only available during physics-enabled play/net sessions");
    UData* origin = check_box(L, 2, UKind::Vector3, "Vector3");
    UData* direction = check_box(L, 3, UKind::Vector3, "Vector3");
    std::vector<Instance*> filter;
    bool has_filter = false;
    if (!lua_isnoneornil(L, 4)) {
        UData* params = check_box(L, 4, UKind::RaycastParams, "RaycastParams");
        has_filter = true;
        if (params->filter_type) {
            filter = params->filter_list; // Include list directly
        } else {
            std::vector<Instance*> excluded;
            for (Instance* f : params->filter_list) {
                std::vector<Instance*> stack{f};
                while (!stack.empty()) {
                    Instance* cur = stack.back(); stack.pop_back();
                    excluded.push_back(cur);
                    for (const auto& c : cur->children) stack.push_back(c.get());
                }
            }
            std::vector<Instance*> stack{&rt->model().root()};
            while (!stack.empty()) {
                Instance* cur = stack.back(); stack.pop_back();
                bool is_excluded = false;
                for (Instance* e : excluded) if (e == cur) { is_excluded = true; break; }
                if (!is_excluded) filter.push_back(cur);
                for (const auto& c : cur->children) stack.push_back(c.get());
            }
        }
    }
    std::array<double, 3> o{origin->n[0], origin->n[1], origin->n[2]};
    std::array<double, 3> d{direction->n[0], direction->n[1], direction->n[2]};
    double max_dist = std::sqrt(d[0] * d[0] + d[1] * d[1] + d[2] * d[2]);
    PhysicsWorld::RayHit hit = rt->impl_->physics->raycast_filtered(
        o, d, max_dist, has_filter ? &filter : nullptr);
    if (!hit.hit) { lua_pushnil(L); return 1; }
    lua_newtable(L);
    push_instance(L, hit.instance);
    lua_setfield(L, -2, "Instance");
    push_vector3(L, hit.point[0], hit.point[1], hit.point[2]);
    lua_setfield(L, -2, "Position");
    push_vector3(L, hit.normal[0], hit.normal[1], hit.normal[2]);
    lua_setfield(L, -2, "Normal");
    lua_pushnumber(L, hit.distance);
    lua_setfield(L, -2, "Distance");
    if (hit.instance) { lua_pushstring(L, hit.instance->class_name.c_str()); lua_setfield(L, -2, "Material"); }
    return 1;
}

int m_raycast_params_new(lua_State* L) {
    UData* u = new_box(L, UKind::RaycastParams, "BloxRaycastParams");
    u->filter_type = 0; // Exclude by default (matches Roblox)
    u->ignore_water = true;
    return 1;
}

int rcp_index(lua_State* L) {
    UData* u = check_box(L, 1, UKind::RaycastParams, "RaycastParams");
    std::string key = luaL_checkstring(L, 2);
    if (key == "FilterDescendantsInstances") {
        lua_newtable(L);
        int i = 1;
        for (Instance* inst : u->filter_list) { push_instance(L, inst); lua_rawseti(L, -2, i++); }
        return 1;
    }
    if (key == "FilterType") { push_enum_item(L, "RaycastFilterType", u->filter_type ? "Include" : "Exclude", u->filter_type); return 1; }
    if (key == "IgnoreWater") { lua_pushboolean(L, u->ignore_water ? 1 : 0); return 1; }
    if (key == "CollisionGroup") { lua_pushstring(L, "Default"); return 1; }
    lua_pushnil(L);
    return 1;
}

int rcp_newindex(lua_State* L) {
    UData* u = check_box(L, 1, UKind::RaycastParams, "RaycastParams");
    std::string key = luaL_checkstring(L, 2);
    if (key == "FilterDescendantsInstances") {
        u->filter_list.clear();
        luaL_checktype(L, 3, LUA_TTABLE);
        int n = static_cast<int>(lua_objlen(L, 3));
        for (int i = 1; i <= n; ++i) {
            lua_rawgeti(L, 3, i);
            if (lua_type(L, -1) == LUA_TUSERDATA) {
                auto* box = static_cast<UData*>(lua_touserdata(L, -1));
                if (box && box->kind == UKind::Instance && box->inst) u->filter_list.push_back(box->inst);
            }
            lua_pop(L, 1);
        }
        return 0;
    }
    if (key == "FilterType") {
        UData* e = check_box(L, 3, UKind::EnumItem, "EnumItem");
        u->filter_type = e->s2 == "Include" ? 1 : 0;
        return 0;
    }
    if (key == "IgnoreWater") { u->ignore_water = lua_toboolean(L, 3) != 0; return 0; }
    luaL_error(L, "RaycastParams has no settable field '%s'", key.c_str());
    return 0;
}


// ------------------------------------------------------------- animation --

struct QuatD { double x, y, z, w; };

QuatD quat_from_rotation(const std::array<double, 9>& r) {
    // Rotation stored row-major: [r00 r01 r02; r10 r11 r12; r20 r21 r22]
    // as (col0, col1, col2) triplets: rotation[0..2] = column 0.
    double m00 = r[0], m10 = r[3], m20 = r[6];
    double m01 = r[1], m11 = r[4], m21 = r[7];
    double m02 = r[2], m12 = r[5], m22 = r[8];
    double trace = m00 + m11 + m22;
    QuatD q;
    if (trace > 0) {
        double s = std::sqrt(trace + 1.0) * 2;
        q = {(m21 - m12) / s, (m02 - m20) / s, (m10 - m01) / s, 0.25 * s};
    } else if (m00 > m11 && m00 > m22) {
        double s = std::sqrt(1.0 + m00 - m11 - m22) * 2;
        q = {0.25 * s, (m01 + m10) / s, (m02 + m20) / s, (m21 - m12) / s};
    } else if (m11 > m22) {
        double s = std::sqrt(1.0 + m11 - m00 - m22) * 2;
        q = {(m01 + m10) / s, 0.25 * s, (m12 + m21) / s, (m02 - m20) / s};
    } else {
        double s = std::sqrt(1.0 + m22 - m00 - m11) * 2;
        q = {(m02 + m20) / s, (m12 + m21) / s, 0.25 * s, (m10 - m01) / s};
    }
    double len = std::sqrt(q.x * q.x + q.y * q.y + q.z * q.z + q.w * q.w);
    if (len > 0) { q.x /= len; q.y /= len; q.z /= len; q.w /= len; }
    return q;
}

std::array<double, 9> rotation_from_quat(const QuatD& q) {
    double x = q.x, y = q.y, z = q.z, w = q.w;
    // Returns columns (c0, c1, c2) matching Variant storage.
    return {
        1 - 2 * (y * y + z * z), 2 * (x * y + z * w), 2 * (x * z - y * w),
        2 * (x * y - z * w), 1 - 2 * (x * x + z * z), 2 * (y * z + x * w),
        2 * (x * z + y * w), 2 * (y * z - x * w), 1 - 2 * (x * x + y * y),
    };
}

QuatD quat_slerp(QuatD a, QuatD b, double t) {
    double dot = a.x * b.x + a.y * b.y + a.z * b.z + a.w * b.w;
    if (dot < 0) { b = {-b.x, -b.y, -b.z, -b.w}; dot = -dot; }
    if (dot > 0.9995) {
        QuatD q{a.x + t * (b.x - a.x), a.y + t * (b.y - a.y), a.z + t * (b.z - a.z), a.w + t * (b.w - a.w)};
        double len = std::sqrt(q.x * q.x + q.y * q.y + q.z * q.z + q.w * q.w);
        if (len > 0) { q.x /= len; q.y /= len; q.z /= len; q.w /= len; }
        return q;
    }
    double theta = std::acos(std::min(1.0, dot));
    double s = std::sin(theta);
    double wa = std::sin((1 - t) * theta) / s, wb = std::sin(t * theta) / s;
    return {wa * a.x + wb * b.x, wa * a.y + wb * b.y, wa * a.z + wb * b.z, wa * a.w + wb * b.w};
}

Variant sample_sequence(Instance* sequence, double t, std::map<std::string, Variant>& out) {
    (void)t;
    std::vector<Instance*> frames;
    for (const auto& child : sequence->children)
        if (child->class_name == "Keyframe") frames.push_back(child.get());
    std::sort(frames.begin(), frames.end(), [](Instance* a, Instance* b) {
        auto time_of = [](Instance* k) {
            auto it = k->properties.find("Time");
            return (it != k->properties.end() && (it->second.type == ValueType::Float64 || it->second.type == ValueType::Float32))
                       ? it->second.float_value : 0.0;
        };
        return time_of(a) < time_of(b);
    });
    out.clear();
    if (frames.empty()) return Variant::make_float64(0);
    auto time_of = [](Instance* k) {
        auto it = k->properties.find("Time");
        return (it != k->properties.end() && (it->second.type == ValueType::Float64 || it->second.type == ValueType::Float32))
                   ? it->second.float_value : 0.0;
    };
    double length = time_of(frames.back());
    auto poses_of = [](Instance* frame, std::map<std::string, Variant>& dst) {
        for (const auto& pose : frame->children) {
            if (pose->class_name != "Pose") continue;
            auto it = pose->properties.find("CFrame");
            if (it != pose->properties.end() && it->second.type == ValueType::CFrame)
                dst[pose->name] = it->second;
        }
    };
    if (t <= time_of(frames.front())) { poses_of(frames.front(), out); return Variant::make_float64(length); }
    if (t >= length) { poses_of(frames.back(), out); return Variant::make_float64(length); }
    for (size_t i = 0; i + 1 < frames.size(); ++i) {
        double t0 = time_of(frames[i]), t1 = time_of(frames[i + 1]);
        if (t < t0 || t > t1) continue;
        double alpha = t1 > t0 ? (t - t0) / (t1 - t0) : 0.0;
        std::map<std::string, Variant> a, b;
        poses_of(frames[i], a);
        poses_of(frames[i + 1], b);
        for (const auto& [name, va] : a) {
            auto it = b.find(name);
            if (it == b.end()) { out[name] = va; continue; }
            const Variant& vb = it->second;
            Variant v;
            v.type = ValueType::CFrame;
            for (int k = 0; k < 3; ++k) v.position[k] = va.position[k] + alpha * (vb.position[k] - va.position[k]);
            QuatD q = quat_slerp(quat_from_rotation(va.rotation), quat_from_rotation(vb.rotation), alpha);
            v.rotation = rotation_from_quat(q);
            out[name] = v;
        }
        for (const auto& [name, vb] : b)
            if (!a.count(name)) out[name] = vb;
        break;
    }
    return Variant::make_float64(length);
}

Instance* find_motor6d(Instance* root, const std::string& joint_name) {
    std::vector<Instance*> stack{root};
    while (!stack.empty()) {
        Instance* cur = stack.back();
        stack.pop_back();
        if (cur->class_name == "Motor6D" && cur->name == joint_name) return cur;
        for (const auto& child : cur->children) stack.push_back(child.get());
    }
    return nullptr;
}

Instance* model_of(Instance* inst) {
    for (Instance* cur = inst; cur; cur = cur->parent)
        if (cur->class_name == "Model") return cur;
    return nullptr;
}

double sequence_length(Instance* sequence) {
    double length = 0;
    for (const auto& child : sequence->children) {
        if (child->class_name != "Keyframe") continue;
        auto it = child->properties.find("Time");
        if (it != child->properties.end() && (it->second.type == ValueType::Float64 || it->second.type == ValueType::Float32))
            length = std::max(length, static_cast<double>(it->second.float_value));
    }
    return length;
}

using AnimTrackState = Runtime::Impl::AnimTrackState;

// -------------------------------------------------- ContextActionService --

int m_cas_bind_action(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* service = check_instance(L, 1);
    if (service->class_name != "ContextActionService")
        luaL_error(L, "BindAction must be called on ContextActionService");
    std::string name = luaL_checkstring(L, 2);
    luaL_checktype(L, 3, LUA_TFUNCTION);
    Runtime::Impl::BoundAction action;
    action.name = name;
    lua_pushvalue(L, 3);
    action.fn_ref = lua_ref(L, -1);
    lua_pop(L, 1);
    for (int i = 5; i <= lua_gettop(L); ++i) {
        if (lua_type(L, i) == LUA_TUSERDATA) {
            UData* u = static_cast<UData*>(lua_touserdata(L, i));
            if (u && u->kind == UKind::EnumItem) action.inputs.push_back(u->s2);
        } else if (lua_isstring(L, i)) {
            action.inputs.push_back(lua_tostring(L, i));
        }
    }
    auto& actions = rt->impl_->bound_actions;
    actions.erase(std::remove_if(actions.begin(), actions.end(),
                                 [&](const Runtime::Impl::BoundAction& a) { return a.name == name; }),
                  actions.end());
    actions.push_back(std::move(action));
    return 0;
}

int m_cas_unbind_action(lua_State* L) {
    Runtime* rt = rt_of(L);
    std::string name = luaL_checkstring(L, 2);
    auto& actions = rt->impl_->bound_actions;
    actions.erase(std::remove_if(actions.begin(), actions.end(),
                                 [&](const Runtime::Impl::BoundAction& a) { return a.name == name; }),
                  actions.end());
    return 0;
}

int m_cas_unbind_all(lua_State* L) {
    rt_of(L)->impl_->bound_actions.clear();
    return 0;
}

bool resume_thread(Runtime* rt, Thread* t); // fwd (defined below)

void fire_bound_actions(Runtime* rt, lua_State* L, const std::string& key_name, bool down) {
    for (const auto& action : rt->impl_->bound_actions) {
        bool match = false;
        for (const auto& input : action.inputs)
            if (input == key_name) { match = true; break; }
        if (!match) continue;
        Thread* t = make_thread(rt, L, "ContextActionService/" + action.name);
        lua_State* co = t->co;
        lua_rawgeti(co, LUA_REGISTRYINDEX, action.fn_ref);
        lua_pushstring(co, action.name.c_str());
        push_enum_item(co, "UserInputState", down ? "Begin" : "End", down ? 0 : 2);
        lua_newtable(co);
        push_enum_item(co, "KeyCode", key_name, 0);
        lua_setfield(co, -2, "KeyCode");
        push_enum_item(co, "UserInputType", "Keyboard", 8);
        lua_setfield(co, -2, "UserInputType");
        push_enum_item(co, "UserInputState", down ? "Begin" : "End", down ? 0 : 2);
        lua_setfield(co, -2, "UserInputState");
        t->pending_args = 3;
        resume_thread(rt, t);
    }
}

// ------------------------------------------------------- PhysicsService --

void set_service_attribute(Runtime* rt, Instance* inst, const std::string& key,
                           const std::string& type, const Json& value) {
    Variant attrs;
    auto it = inst->properties.find("Attributes");
    if (it != inst->properties.end() && it->second.type == ValueType::Attributes)
        attrs = it->second;
    else {
        attrs.type = ValueType::Attributes;
        attrs.complex_value = Json::object();
    }
    Json entry = Json::object();
    entry.set("type", Json(type));
    entry.set("value", value);
    attrs.complex_value.set(key, std::move(entry));
    std::string error;
    rt->model().set_property(*inst, "Attributes", attrs, error);
}

bool get_service_bool_attribute(Instance* inst, const std::string& key, bool fallback) {
    auto it = inst->properties.find("Attributes");
    if (it == inst->properties.end() || it->second.type != ValueType::Attributes) return fallback;
    const Json* entry = it->second.complex_value.find(key);
    if (!entry || !entry->find("value")) return fallback;
    return entry->find("value")->as_bool();
}

Instance* check_physics_service(lua_State* L, int idx) {
    Instance* inst = check_instance(L, idx);
    if (inst->class_name != "PhysicsService")
        luaL_error(L, "this method is only available on PhysicsService");
    return inst;
}

int m_physics_register_group(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* service = check_physics_service(L, 1);
    std::string name = luaL_checkstring(L, 2);
    // Recorded as a service attribute so the physics builder (which
    // reads the same DataModel) sees the registry.
    set_service_attribute(rt, service, "group:" + name, "String", Json(name));
    return 0;
}

int m_physics_set_part_group(lua_State* L) {
    Runtime* rt = rt_of(L);
    check_physics_service(L, 1);
    Instance* part = check_instance(L, 2);
    std::string name = luaL_checkstring(L, 3);
    std::string error;
    if (!rt->model().set_property(*part, "CollisionGroup", Variant::make_string(name), error))
        luaL_error(L, "%s", error.c_str());
    return 0;
}

std::string collision_pair_key(const std::string& a, const std::string& b) {
    return a < b ? "cg:" + a + "|" + b : "cg:" + b + "|" + a;
}

int m_physics_set_collidable(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* service = check_physics_service(L, 1);
    std::string a = luaL_checkstring(L, 2);
    std::string b = luaL_checkstring(L, 3);
    bool collidable = lua_toboolean(L, 4) != 0;
    set_service_attribute(rt, service, collision_pair_key(a, b), "Bool", Json(collidable));
    return 0;
}

int m_physics_are_collidable(lua_State* L) {
    Instance* service = check_physics_service(L, 1);
    std::string a = luaL_checkstring(L, 2);
    std::string b = luaL_checkstring(L, 3);
    if (a == b) { lua_pushboolean(L, 1); return 1; }
    lua_pushboolean(L, get_service_bool_attribute(service, collision_pair_key(a, b), true) ? 1 : 0);
    return 1;
}

// -------------------------------------------------- PathfindingService --
// Grid A* over BasePart AABBs (documented approximation: no navmesh,
// 2-stud cells, floors are part top surfaces, jump links up to 3.2
// studs when AgentCanJump). Character models are excluded.

struct PathPart {
    double minx, maxx, miny, maxy, minz, maxz;
};

int m_pathfinding_create_path(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* service = check_instance(L, 1);
    if (service->class_name != "PathfindingService")
        luaL_error(L, "CreatePath must be called on PathfindingService");
    auto state = std::make_unique<Runtime::Impl::PathState>();
    if (lua_istable(L, 2)) {
        lua_getfield(L, 2, "AgentRadius");
        if (lua_isnumber(L, -1)) state->agent_radius = lua_tonumber(L, -1);
        lua_pop(L, 1);
        lua_getfield(L, 2, "AgentCanJump");
        if (!lua_isnil(L, -1)) state->can_jump = lua_toboolean(L, -1) != 0;
        lua_pop(L, 1);
    }
    Runtime::Impl::PathState* raw = state.get();
    rt->impl_->paths.push_back(std::move(state));
    UData* u = new_box(L, UKind::PathResult, "BloxPath");
    u->extra = raw;
    return 1;
}

int m_path_compute(lua_State* L) {
    Runtime* rt = rt_of(L);
    auto* u = check_box(L, 1, UKind::PathResult, "Path");
    auto* path = static_cast<Runtime::Impl::PathState*>(u->extra);
    UData* from = check_box(L, 2, UKind::Vector3, "Vector3");
    UData* to = check_box(L, 3, UKind::Vector3, "Vector3");
    path->waypoints.clear();
    path->actions.clear();
    path->status = "NoPath";

    // Collect collidable part AABBs (excluding character rigs).
    std::vector<PathPart> parts;
    Instance* workspace = nullptr;
    for (const auto& child : rt->model().root().children)
        if (child->class_name == "Workspace") workspace = child.get();
    if (!workspace) return 0;
    std::vector<std::pair<Instance*, bool>> stack{{workspace, false}};
    while (!stack.empty()) {
        auto [inst, in_character] = stack.back();
        stack.pop_back();
        bool character = in_character;
        if (inst->class_name == "Model") {
            for (const auto& c : inst->children)
                if (c->class_name == "Humanoid") character = true;
        }
        if (!character && DataModel::is_base_part(inst->class_name)) {
            auto cit = inst->properties.find("CanCollide");
            bool can_collide = cit == inst->properties.end() || cit->second.type != ValueType::Bool ||
                               cit->second.bool_value;
            auto cf_it = inst->properties.find("CFrame");
            auto sz_it = inst->properties.find("Size");
            if (can_collide && cf_it != inst->properties.end() && sz_it != inst->properties.end() &&
                cf_it->second.type == ValueType::CFrame && sz_it->second.type == ValueType::Vector3) {
                const Variant& cf = cf_it->second;
                const Variant& sz = sz_it->second;
                PathPart pp{1e18, -1e18, 1e18, -1e18, 1e18, -1e18};
                for (int corner = 0; corner < 8; ++corner) {
                    double lx = (corner & 1 ? 1 : -1) * sz.vec3[0] / 2;
                    double ly = (corner & 2 ? 1 : -1) * sz.vec3[1] / 2;
                    double lz = (corner & 4 ? 1 : -1) * sz.vec3[2] / 2;
                    double wx = cf.position[0] + cf.rotation[0] * lx + cf.rotation[1] * ly + cf.rotation[2] * lz;
                    double wy = cf.position[1] + cf.rotation[3] * lx + cf.rotation[4] * ly + cf.rotation[5] * lz;
                    double wz = cf.position[2] + cf.rotation[6] * lx + cf.rotation[7] * ly + cf.rotation[8] * lz;
                    pp.minx = std::min(pp.minx, wx); pp.maxx = std::max(pp.maxx, wx);
                    pp.miny = std::min(pp.miny, wy); pp.maxy = std::max(pp.maxy, wy);
                    pp.minz = std::min(pp.minz, wz); pp.maxz = std::max(pp.maxz, wz);
                }
                parts.push_back(pp);
            }
        }
        for (const auto& child : inst->children) stack.push_back({child.get(), character});
    }

    const double cell = 2.0;
    double minx = std::min(from->n[0], to->n[0]) - 26, maxx = std::max(from->n[0], to->n[0]) + 26;
    double minz = std::min(from->n[2], to->n[2]) - 26, maxz = std::max(from->n[2], to->n[2]) + 26;
    int nx = static_cast<int>((maxx - minx) / cell) + 1;
    int nz = static_cast<int>((maxz - minz) / cell) + 1;
    double cap_y = std::max(from->n[1], to->n[1]) + 3.0;
    // Cells test area overlap (a 1-stud wall must block the cells it
    // crosses even when no cell centre lands inside it).
    auto floor_at = [&](double wx, double wz, double& floor, bool& blocked) {
        const double half = cell / 2;
        auto overlaps = [&](const PathPart& pp) {
            return pp.maxx >= wx - half && pp.minx <= wx + half &&
                   pp.maxz >= wz - half && pp.minz <= wz + half;
        };
        floor = -1e18;
        for (const PathPart& pp : parts) {
            if (overlaps(pp) && pp.maxy <= cap_y)
                floor = std::max(floor, pp.maxy);
        }
        if (floor < -1e17) { blocked = true; return; }
        blocked = false;
        for (const PathPart& pp : parts) {
            if (overlaps(pp) &&
                pp.miny < floor + 4.5 && pp.maxy > floor + 0.25 && std::abs(pp.maxy - floor) > 0.01) {
                blocked = true;
                return;
            }
        }
    };
    std::vector<double> floors(static_cast<size_t>(nx * nz), -1e18);
    std::vector<char> blocks(static_cast<size_t>(nx * nz), 1);
    for (int iz = 0; iz < nz; ++iz)
        for (int ix = 0; ix < nx; ++ix) {
            double f; bool b;
            floor_at(minx + (ix + 0.5) * cell, minz + (iz + 0.5) * cell, f, b);
            floors[static_cast<size_t>(iz * nx + ix)] = f;
            blocks[static_cast<size_t>(iz * nx + ix)] = b ? 1 : 0;
        }
    auto index_of = [&](double wx, double wz) {
        int ix = static_cast<int>((wx - minx) / cell), iz = static_cast<int>((wz - minz) / cell);
        ix = std::max(0, std::min(nx - 1, ix));
        iz = std::max(0, std::min(nz - 1, iz));
        return iz * nx + ix;
    };
    int start = index_of(from->n[0], from->n[2]);
    int goal = index_of(to->n[0], to->n[2]);
    if (blocks[static_cast<size_t>(start)] || blocks[static_cast<size_t>(goal)]) return 0;
    // A*.
    std::vector<double> cost(static_cast<size_t>(nx * nz), 1e18);
    std::vector<int> came(static_cast<size_t>(nx * nz), -1);
    std::vector<int> action_to(static_cast<size_t>(nx * nz), 0);
    std::vector<char> closed(static_cast<size_t>(nx * nz), 0);
    auto heuristic = [&](int idx) {
        int dx = std::abs(idx % nx - goal % nx), dz = std::abs(idx / nx - goal / nx);
        return static_cast<double>(dx + dz);
    };
    std::vector<int> open{start};
    cost[static_cast<size_t>(start)] = 0;
    bool reached = false;
    const int dirs[4][2] = {{1, 0}, {-1, 0}, {0, 1}, {0, -1}};
    while (!open.empty()) {
        size_t best = 0;
        double best_f = 1e18;
        for (size_t i = 0; i < open.size(); ++i) {
            double f = cost[static_cast<size_t>(open[i])] + heuristic(open[i]);
            if (f < best_f) { best_f = f; best = i; }
        }
        int cur = open[best];
        open.erase(open.begin() + static_cast<long>(best));
        if (closed[static_cast<size_t>(cur)]) continue;
        closed[static_cast<size_t>(cur)] = 1;
        if (cur == goal) { reached = true; break; }
        int cx = cur % nx, cz = cur / nx;
        for (const auto& d : dirs) {
            int qx = cx + d[0], qz = cz + d[1];
            if (qx < 0 || qz < 0 || qx >= nx || qz >= nz) continue;
            int nb = qz * nx + qx;
            if (blocks[static_cast<size_t>(nb)] || closed[static_cast<size_t>(nb)]) continue;
            double rise = floors[static_cast<size_t>(nb)] - floors[static_cast<size_t>(cur)];
            int action = 0;
            if (rise > 1.2) {
                if (path->can_jump && rise <= 3.2) action = 1;
                else continue;
            }
            if (rise < -6.0) continue; // no lethal drops
            double nc = cost[static_cast<size_t>(cur)] + 1.0;
            if (nc < cost[static_cast<size_t>(nb)]) {
                cost[static_cast<size_t>(nb)] = nc;
                came[static_cast<size_t>(nb)] = cur;
                action_to[static_cast<size_t>(nb)] = action;
                open.push_back(nb);
            }
        }
    }
    if (!reached) return 0;
    std::vector<int> chain;
    for (int cur = goal; cur != -1; cur = came[static_cast<size_t>(cur)]) chain.push_back(cur);
    std::reverse(chain.begin(), chain.end());
    // Simplify: keep direction changes, jump points and the goal.
    for (size_t i = 0; i < chain.size(); ++i) {
        bool keep = i + 1 == chain.size() || action_to[static_cast<size_t>(chain[i])] == 1;
        if (!keep && i > 0) {
            int dx1 = chain[i] % nx - chain[i - 1] % nx, dz1 = chain[i] / nx - chain[i - 1] / nx;
            int dx2 = chain[i + 1] % nx - chain[i] % nx, dz2 = chain[i + 1] / nx - chain[i] / nx;
            keep = dx1 != dx2 || dz1 != dz2;
        }
        if (!keep && i > 0) continue;
        int idx = chain[i];
        path->waypoints.push_back({minx + (idx % nx + 0.5) * cell,
                                   floors[static_cast<size_t>(idx)],
                                   minz + (idx / nx + 0.5) * cell});
        path->actions.push_back(action_to[static_cast<size_t>(idx)]);
    }
    path->waypoints.push_back({to->n[0], floors[static_cast<size_t>(goal)], to->n[2]});
    path->actions.push_back(0);
    path->status = "Success";
    return 0;
}

int m_path_get_waypoints(lua_State* L) {
    auto* u = check_box(L, 1, UKind::PathResult, "Path");
    auto* path = static_cast<Runtime::Impl::PathState*>(u->extra);
    lua_newtable(L);
    for (size_t i = 0; i < path->waypoints.size(); ++i) {
        lua_newtable(L);
        push_vector3(L, path->waypoints[i][0], path->waypoints[i][1], path->waypoints[i][2]);
        lua_setfield(L, -2, "Position");
        push_enum_item(L, "PathWaypointAction", path->actions[i] ? "Jump" : "Walk", path->actions[i]);
        lua_setfield(L, -2, "Action");
        lua_pushstring(L, "");
        lua_setfield(L, -2, "Label");
        lua_rawseti(L, -2, static_cast<int>(i) + 1);
    }
    return 1;
}

int path_index(lua_State* L) {
    auto* u = check_box(L, 1, UKind::PathResult, "Path");
    auto* path = static_cast<Runtime::Impl::PathState*>(u->extra);
    std::string key = luaL_checkstring(L, 2);
    if (key == "Status") {
        int64_t value = path->status == "Success" ? 0 : 3;
        push_enum_item(L, "PathStatus", path->status, value);
        return 1;
    }
    lua_getfield(L, LUA_REGISTRYINDEX, "blox_path_methods");
    lua_pushstring(L, key.c_str());
    lua_rawget(L, -2);
    lua_remove(L, -2);
    return 1;
}

namespace {
const std::map<std::string, int64_t>& humanoid_state_values() {
    static const std::map<std::string, int64_t> values = {
        {"FallingDown", 0}, {"Running", 1}, {"RunningNoPhysics", 2}, {"Climbing", 3},
        {"StrafingNoPhysics", 4}, {"Ragdoll", 5}, {"GettingUp", 6}, {"Jumping", 7},
        {"Landed", 8}, {"Flying", 9}, {"Freefall", 10}, {"Seated", 11},
        {"PlatformStanding", 12}, {"Dead", 13}, {"Swimming", 14}, {"Physics", 15}, {"None", 16}};
    return values;
}
} // namespace

LuaArg enum_arg(const char* type, const std::string& name) {
    LuaArg a;
    a.kind = 5;
    a.box.kind = UKind::EnumItem;
    a.box.s1 = type;
    a.box.s2 = name;
    auto it = humanoid_state_values().find(name);
    a.box.n[0] = it != humanoid_state_values().end() ? static_cast<double>(it->second) : 16.0;
    return a;
}

int m_humanoid_get_state(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* humanoid = check_instance(L, 1);
    auto it = rt->impl_->humanoid_states.find(humanoid);
    std::string state = it != rt->impl_->humanoid_states.end() ? it->second : "Running";
    push_enum_item(L, "HumanoidStateType", state,
                   humanoid_state_values().count(state) ? humanoid_state_values().at(state) : 16);
    return 1;
}

int m_humanoid_change_state(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* humanoid = check_instance(L, 1);
    std::string state;
    if (lua_type(L, 2) == LUA_TUSERDATA) {
        UData* u = check_box(L, 2, UKind::EnumItem, "EnumItem");
        state = u->s2;
    } else {
        state = luaL_checkstring(L, 2);
    }
    if (!humanoid_state_values().count(state))
        luaL_error(L, "unknown HumanoidStateType '%s'", state.c_str());
    rt->set_humanoid_state(humanoid, state);
    return 0;
}

int m_animator_load_animation(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* animator = check_instance(L, 1);
    if (animator->class_name == "Humanoid") {
        // Humanoid:LoadAnimation delegates to its Animator child.
        Instance* found = nullptr;
        for (const auto& child : animator->children)
            if (child->class_name == "Animator") { found = child.get(); break; }
        if (!found) luaL_error(L, "Humanoid has no Animator child to load animations through");
        animator = found;
    }
    if (animator->class_name != "Animator")
        luaL_error(L, "LoadAnimation must be called on an Animator");
    Instance* anim = check_instance(L, 2);
    Instance* sequence = nullptr;
    if (anim->class_name == "KeyframeSequence") {
        sequence = anim;
    } else {
        for (const auto& child : anim->children)
            if (child->class_name == "KeyframeSequence") { sequence = child.get(); break; }
    }
    if (!sequence) {
        std::string id;
        auto it = anim->properties.find("AnimationId");
        if (it != anim->properties.end()) id = it->second.string_value;
        luaL_error(L, "LoadAnimation: animation has no KeyframeSequence child%s; asset-id animations cannot be fetched offline",
                   id.empty() ? "" : (" ('" + id + "')").c_str());
    }
    auto state = std::make_unique<AnimTrackState>();
    state->animator = animator;
    state->sequence = sequence;
    auto loop_it = sequence->properties.find("Loop");
    state->looped = loop_it != sequence->properties.end() && loop_it->second.type == ValueType::Bool &&
                    loop_it->second.bool_value;
    AnimTrackState* raw = state.get();
    rt->impl_->anim_tracks.push_back(std::move(state));
    UData* u = new_box(L, UKind::AnimationTrack, "BloxAnimationTrack");
    u->extra = raw;
    return 1;
}

int track_play(lua_State* L) {
    auto* u = check_box(L, 1, UKind::AnimationTrack, "AnimationTrack");
    auto* track = static_cast<AnimTrackState*>(u->extra);
    if (track->time_pos >= sequence_length(track->sequence)) track->time_pos = 0;
    track->playing = true;
    track->stopped = false;
    track->has_played = true;
    return 0;
}

int track_stop(lua_State* L) {
    Runtime* rt = rt_of(L);
    auto* u = check_box(L, 1, UKind::AnimationTrack, "AnimationTrack");
    auto* track = static_cast<AnimTrackState*>(u->extra);
    track->playing = false;
    track->stopped = true;
    // Release joints back to the rig's rest (identity) transforms.
    for (Instance* motor : track->driven) {
        std::string error;
        rt->model().set_property(*motor, "Transform", Variant::make_cframe(0, 0, 0), error);
    }
    track->driven.clear();
    return 0;
}

int track_adjust_weight(lua_State* L) {
    auto* u = check_box(L, 1, UKind::AnimationTrack, "AnimationTrack");
    auto* track = static_cast<AnimTrackState*>(u->extra);
    track->weight = std::max(0.0, luaL_checknumber(L, 2));
    return 0;
}

int track_adjust_speed(lua_State* L) {
    auto* u = check_box(L, 1, UKind::AnimationTrack, "AnimationTrack");
    auto* track = static_cast<AnimTrackState*>(u->extra);
    track->speed = luaL_checknumber(L, 2);
    return 0;
}

int track_index(lua_State* L) {
    auto* u = check_box(L, 1, UKind::AnimationTrack, "AnimationTrack");
    auto* track = static_cast<AnimTrackState*>(u->extra);
    std::string key = luaL_checkstring(L, 2);
    if (key == "TimePosition") { lua_pushnumber(L, track->time_pos); return 1; }
    if (key == "Speed") { lua_pushnumber(L, track->speed); return 1; }
    if (key == "IsPlaying") { lua_pushboolean(L, track->playing ? 1 : 0); return 1; }
    if (key == "Length") { lua_pushnumber(L, sequence_length(track->sequence)); return 1; }
    if (key == "Looped") { lua_pushboolean(L, track->looped ? 1 : 0); return 1; }
    if (key == "Weight") { lua_pushnumber(L, track->weight); return 1; }
    lua_getfield(L, LUA_REGISTRYINDEX, "blox_track_methods");
    lua_pushstring(L, key.c_str());
    lua_rawget(L, -2);
    lua_remove(L, -2);
    return 1;
}

int track_newindex(lua_State* L) {
    auto* u = check_box(L, 1, UKind::AnimationTrack, "AnimationTrack");
    auto* track = static_cast<AnimTrackState*>(u->extra);
    std::string key = luaL_checkstring(L, 2);
    if (key == "TimePosition") {
        track->time_pos = std::max(0.0, luaL_checknumber(L, 3));
        return 0;
    }
    if (key == "Looped") { track->looped = lua_toboolean(L, 3) != 0; return 0; }
    if (key == "Weight") { track->weight = std::max(0.0, luaL_checknumber(L, 3)); return 0; }
    luaL_error(L, "AnimationTrack.%s is not writable", key.c_str());
    return 0;
}

void evaluate_anim_tracks(Runtime* rt, double dt) {
    // Weighted blending (Phase 15): every non-stopped track that has
    // been played contributes its sampled pose per joint; contributions
    // are combined by weight (positions averaged, rotations accumulated
    // as sign-aligned quaternion sums, then normalised).
    struct Accum {
        Instance* motor = nullptr;
        double pos[3] = {0, 0, 0};
        QuatD quat{0, 0, 0, 0};
        bool first = true;
        double wsum = 0;
    };
    std::map<Instance*, Accum> writes;
    for (auto& track_ptr : rt->impl_->anim_tracks) {
        AnimTrackState* track = track_ptr.get();
        if (!track->sequence) continue;
        if (track->playing) {
            double length = sequence_length(track->sequence);
            track->time_pos += dt * track->speed;
            if (length > 0 && track->time_pos >= length) {
                if (track->looped) {
                    track->time_pos = std::fmod(track->time_pos, length);
                } else {
                    track->time_pos = length;
                    track->playing = false;
                }
            }
        }
        if (track->stopped || !track->has_played || track->weight <= 0) continue;
        Instance* model = model_of(track->animator);
        if (!model) continue;
        std::map<std::string, Variant> sampled;
        sample_sequence(track->sequence, track->time_pos, sampled);
        track->driven.clear();
        for (const auto& [joint_name, cf] : sampled) {
            Instance* motor = find_motor6d(model, joint_name);
            if (!motor) continue;
            track->driven.push_back(motor);
            Accum& acc = writes[motor];
            acc.motor = motor;
            double w = track->weight;
            for (int k = 0; k < 3; ++k) acc.pos[k] += cf.position[k] * w;
            QuatD q = quat_from_rotation(cf.rotation);
            if (acc.first) {
                acc.quat = q;
                acc.first = false;
            } else {
                double dot = acc.quat.x * q.x + acc.quat.y * q.y + acc.quat.z * q.z + acc.quat.w * q.w;
                double sign = dot < 0 ? -1.0 : 1.0;
                acc.quat.x += sign * q.x * w;
                acc.quat.y += sign * q.y * w;
                acc.quat.z += sign * q.z * w;
                acc.quat.w += sign * q.w * w;
            }
            if (acc.wsum == 0) {
                // First contribution was stored unweighted above.
                acc.quat.x *= w;
                acc.quat.y *= w;
                acc.quat.z *= w;
                acc.quat.w *= w;
            }
            acc.wsum += w;
        }
    }
    for (auto& [motor, acc] : writes) {
        if (acc.wsum <= 0) continue;
        Variant v;
        v.type = ValueType::CFrame;
        for (int k = 0; k < 3; ++k) v.position[k] = acc.pos[k] / acc.wsum;
        double len = std::sqrt(acc.quat.x * acc.quat.x + acc.quat.y * acc.quat.y +
                               acc.quat.z * acc.quat.z + acc.quat.w * acc.quat.w);
        QuatD q = acc.quat;
        if (len > 0) { q.x /= len; q.y /= len; q.z /= len; q.w /= len; }
        v.rotation = rotation_from_quat(q);
        std::string error;
        rt->model().set_property(*motor, "Transform", v, error);
    }
}


// --------------------------------------------------------------- terrain --

Instance* check_terrain(lua_State* L, int idx) {
    Instance* inst = check_instance(L, idx);
    if (inst->class_name != "Terrain")
        luaL_error(L, "this method is only available on the Terrain instance");
    return inst;
}

uint16_t material_from_enum(lua_State* L, int idx) {
    UData* u = check_box(L, idx, UKind::EnumItem, "EnumItem");
    int64_t value = static_cast<int64_t>(u->n[0]);
    if (value == 1792) return 0; // Enum.Material.Air erases
    return static_cast<uint16_t>(value);
}

int m_terrain_fill_block(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* terrain = check_terrain(L, 1);
    UData* cf = check_box(L, 2, UKind::CFrame, "CFrame");
    UData* size = check_box(L, 3, UKind::Vector3, "Vector3");
    uint16_t material = material_from_enum(L, 4);
    TerrainGrid grid;
    std::string error;
    if (!terrain_load(*terrain, grid, error)) luaL_error(L, "%s", error.c_str());
    terrain_fill_block(grid, {cf->n[0], cf->n[1], cf->n[2]},
                       {size->n[0], size->n[1], size->n[2]}, material);
    if (!terrain_store(rt->model(), *terrain, grid, error)) luaL_error(L, "%s", error.c_str());
    return 0;
}

int m_terrain_fill_ball(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* terrain = check_terrain(L, 1);
    UData* pos = check_box(L, 2, UKind::Vector3, "Vector3");
    double radius = luaL_checknumber(L, 3);
    uint16_t material = material_from_enum(L, 4);
    TerrainGrid grid;
    std::string error;
    if (!terrain_load(*terrain, grid, error)) luaL_error(L, "%s", error.c_str());
    terrain_fill_ball(grid, {pos->n[0], pos->n[1], pos->n[2]}, radius, material);
    if (!terrain_store(rt->model(), *terrain, grid, error)) luaL_error(L, "%s", error.c_str());
    return 0;
}

int m_terrain_clear(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* terrain = check_terrain(L, 1);
    TerrainGrid grid;
    std::string error;
    if (!terrain_store(rt->model(), *terrain, grid, error)) luaL_error(L, "%s", error.c_str());
    return 0;
}

int m_terrain_read_voxels(lua_State* L) {
    Instance* terrain = check_terrain(L, 1);
    UData* cf = check_box(L, 2, UKind::CFrame, "CFrame");
    UData* size = check_box(L, 3, UKind::Vector3, "Vector3");
    TerrainGrid grid;
    std::string error;
    if (!terrain_load(*terrain, grid, error)) luaL_error(L, "%s", error.c_str());
    int nx = std::max(1, static_cast<int>(std::round(size->n[0] / kVoxelSize)));
    int ny = std::max(1, static_cast<int>(std::round(size->n[1] / kVoxelSize)));
    int nz = std::max(1, static_cast<int>(std::round(size->n[2] / kVoxelSize)));
    if (nx > 32 || ny > 32 || nz > 32) luaL_error(L, "ReadVoxels region too large (max 32 voxels per axis)");
    int ox = static_cast<int>(std::floor((cf->n[0] - size->n[0] / 2) / kVoxelSize));
    int oy = static_cast<int>(std::floor((cf->n[1] - size->n[1] / 2) / kVoxelSize));
    int oz = static_cast<int>(std::floor((cf->n[2] - size->n[2] / 2) / kVoxelSize));
    // Build both 3D tables bottom-up with explicit stack discipline.
    lua_newtable(L); // materials [x]
    lua_newtable(L); // occupancies [x]  (stack: M O)
    for (int x = 0; x < nx; ++x) {
        lua_newtable(L); // Mx
        lua_newtable(L); // Ox        (stack: M O Mx Ox)
        for (int y = 0; y < ny; ++y) {
            lua_newtable(L); // Mxy
            lua_newtable(L); // Oxy   (stack: M O Mx Ox Mxy Oxy)
            for (int z = 0; z < nz; ++z) {
                auto it = grid.cells.find({ox + x, oy + y, oz + z});
                uint16_t material = it == grid.cells.end() ? 1792 : it->second.first;
                double occ = it == grid.cells.end() ? 0.0 : it->second.second / 255.0;
                static const std::map<int64_t, const char*> kMaterialNames = {
                    {256, "Plastic"}, {272, "SmoothPlastic"}, {288, "Neon"}, {512, "Wood"},
                    {528, "WoodPlanks"}, {784, "Marble"}, {800, "Slate"}, {804, "CrackedLava"},
                    {816, "Concrete"}, {832, "Granite"}, {848, "Brick"}, {1040, "CorrodedMetal"},
                    {1056, "DiamondPlate"}, {1072, "Foil"}, {1088, "Metal"}, {1280, "Grass"},
                    {1284, "LeafyGrass"}, {1296, "Fabric"}, {1312, "Sand"}, {1328, "Snow"},
                    {1536, "Ice"}, {1540, "Glacier"}, {1568, "Glass"}, {1584, "ForceField"},
                    {1792, "Air"}, {2048, "Water"}};
                auto name_it = kMaterialNames.find(material);
                push_enum_item(L, "Material", name_it != kMaterialNames.end() ? name_it->second : "Custom", material);
                lua_rawseti(L, -3, z + 1); // into Mxy
                lua_pushnumber(L, occ);
                lua_rawseti(L, -2, z + 1); // into Oxy
            }
            lua_rawseti(L, -4, y + 1); // Mxy into Mx
            lua_rawseti(L, -2, y + 1); // Oxy into Ox
        }
        lua_rawseti(L, -4, x + 1); // Mx into M
        lua_rawseti(L, -2, x + 1); // Ox into O
    }
    return 2;
}


// -- Phase 26: Region3 + extended Terrain APIs -------------------------

void push_region3(lua_State* L, std::array<double, 3> mn, std::array<double, 3> mx) {
    UData* u = new_box(L, UKind::Region3, "BloxRegion3");
    u->n = {mn[0], mn[1], mn[2], mx[0], mx[1], mx[2], 0, 0, 0, 0, 0, 0};
}
void region3_bounds(lua_State* L, int idx, std::array<double, 3>& mn, std::array<double, 3>& mx) {
    UData* u = check_box(L, idx, UKind::Region3, "Region3");
    mn = {u->n[0], u->n[1], u->n[2]}; mx = {u->n[3], u->n[4], u->n[5]};
}
int region3_new(lua_State* L) {
    auto* a = check_box(L, 1, UKind::Vector3, "Vector3");
    auto* b = check_box(L, 2, UKind::Vector3, "Vector3");
    push_region3(L, {std::min(a->n[0], b->n[0]), std::min(a->n[1], b->n[1]), std::min(a->n[2], b->n[2])},
                    {std::max(a->n[0], b->n[0]), std::max(a->n[1], b->n[1]), std::max(a->n[2], b->n[2])});
    return 1;
}
int region3_expand(lua_State* L) {
    UData* self = check_box(L, lua_upvalueindex(1), UKind::Region3, "Region3");
    double res = lua_isnumber(L, 2) ? lua_tonumber(L, 2) : luaL_checknumber(L, 1);
    if (!(res > 0)) luaL_error(L, "ExpandToGrid resolution must be positive");
    std::array<double, 3> lo, hi;
    for (int i = 0; i < 3; ++i) {
        lo[i] = std::floor(self->n[i] / res) * res;
        hi[i] = std::ceil(self->n[3 + i] / res) * res;
    }
    push_region3(L, lo, hi);
    return 1;
}
int region3_index(lua_State* L) {
    UData* u = check_box(L, 1, UKind::Region3, "Region3");
    std::string key = luaL_checkstring(L, 2);
    std::array<double, 3> mn{u->n[0], u->n[1], u->n[2]}, mx{u->n[3], u->n[4], u->n[5]};
    if (key == "Min") { push_vector3(L, mn[0], mn[1], mn[2]); return 1; }
    if (key == "Max") { push_vector3(L, mx[0], mx[1], mx[2]); return 1; }
    if (key == "Size") { push_vector3(L, mx[0]-mn[0], mx[1]-mn[1], mx[2]-mn[2]); return 1; }
    if (key == "CFrame") {
        push_cframe(L, Variant::make_cframe((mn[0]+mx[0])/2, (mn[1]+mx[1])/2, (mn[2]+mx[2])/2));
        return 1;
    }
    if (key == "ExpandToGrid") {
        lua_pushvalue(L, 1);
        lua_pushcclosure(L, region3_expand, "ExpandToGrid", 1);
        return 1;
    }
    lua_pushnil(L); return 1;
}

int m_terrain_fill_cylinder(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* terrain = check_terrain(L, 1);
    UData* cf = check_box(L, 2, UKind::CFrame, "CFrame");
    double radius = luaL_checknumber(L, 3);
    double height = luaL_checknumber(L, 4);
    uint16_t material = material_from_enum(L, 5);
    TerrainGrid grid; std::string error;
    if (!terrain_load(*terrain, grid, error)) luaL_error(L, "%s", error.c_str());
    terrain_fill_cylinder(grid, {cf->n[0], cf->n[1], cf->n[2]}, radius, height, material);
    if (!terrain_store(rt->model(), *terrain, grid, error)) luaL_error(L, "%s", error.c_str());
    return 0;
}
int m_terrain_fill_wedge(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* terrain = check_terrain(L, 1);
    UData* cf = check_box(L, 2, UKind::CFrame, "CFrame");
    UData* size = check_box(L, 3, UKind::Vector3, "Vector3");
    uint16_t material = material_from_enum(L, 4);
    TerrainGrid grid; std::string error;
    if (!terrain_load(*terrain, grid, error)) luaL_error(L, "%s", error.c_str());
    terrain_fill_wedge(grid, {cf->n[0], cf->n[1], cf->n[2]}, {size->n[0], size->n[1], size->n[2]}, material);
    if (!terrain_store(rt->model(), *terrain, grid, error)) luaL_error(L, "%s", error.c_str());
    return 0;
}
static void region_center_size(lua_State* L, int idx, std::array<double, 3>& center, std::array<double, 3>& size) {
    std::array<double, 3> mn, mx; region3_bounds(L, idx, mn, mx);
    for (int i = 0; i < 3; ++i) { center[i] = (mn[i]+mx[i])/2; size[i] = mx[i]-mn[i]; }
}
int m_terrain_fill_region(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* terrain = check_terrain(L, 1);
    std::array<double, 3> center, size; region_center_size(L, 2, center, size);
    double res = luaL_checknumber(L, 3);
    if (res != 4.0) luaL_error(L, "FillRegion: only resolution 4 is supported");
    uint16_t material = material_from_enum(L, 4);
    TerrainGrid grid; std::string error;
    if (!terrain_load(*terrain, grid, error)) luaL_error(L, "%s", error.c_str());
    terrain_fill_block(grid, center, size, material);
    if (!terrain_store(rt->model(), *terrain, grid, error)) luaL_error(L, "%s", error.c_str());
    return 0;
}
int m_terrain_write_voxels(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* terrain = check_terrain(L, 1);
    std::array<double, 3> center, size; region_center_size(L, 2, center, size);
    double res = luaL_checknumber(L, 3);
    if (res != 4.0) luaL_error(L, "WriteVoxels: only resolution 4 is supported");
    // args 4/5: materials/occupancies [x][y][z]
    TerrainGrid grid; std::string error;
    if (!terrain_load(*terrain, grid, error)) luaL_error(L, "%s", error.c_str());
    int ox = static_cast<int>(std::floor((center[0]-size[0]/2)/kVoxelSize));
    int oy = static_cast<int>(std::floor((center[1]-size[1]/2)/kVoxelSize));
    int oz = static_cast<int>(std::floor((center[2]-size[2]/2)/kVoxelSize));
    lua_pushvalue(L, 4); // materials
    for (int x = 1; ; ++x) {
        lua_rawgeti(L, -1, x); if (lua_isnil(L, -1)) { lua_pop(L, 1); break; }
        for (int y = 1; ; ++y) {
            lua_rawgeti(L, -1, y); if (lua_isnil(L, -1)) { lua_pop(L, 1); break; }
            for (int z = 1; ; ++z) {
                lua_rawgeti(L, -1, z); if (lua_isnil(L, -1)) { lua_pop(L, 1); break; }
                uint16_t mat = material_from_enum(L, -1);
                lua_pop(L, 1);
                // occupancy [x][y][z]
                lua_pushvalue(L, 5); lua_rawgeti(L, -1, x); lua_rawgeti(L, -1, y); lua_rawgeti(L, -1, z);
                double occ = lua_isnumber(L, -1) ? lua_tonumber(L, -1) : 0.0;
                lua_pop(L, 4);
                auto key = std::make_tuple(ox+x-1, oy+y-1, oz+z-1);
                if (occ <= 0.0 || mat == 0) grid.cells.erase(key);
                else grid.cells[key] = {mat, static_cast<uint8_t>(std::max(0.0, std::min(1.0, occ)) * 255)};
            }
            lua_pop(L, 1);
        }
        lua_pop(L, 1);
    }
    lua_pop(L, 1);
    if (!terrain_store(rt->model(), *terrain, grid, error)) luaL_error(L, "%s", error.c_str());
    return 0;
}
int m_terrain_copy_region(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* terrain = check_terrain(L, 1);
    std::array<double, 3> center, size; region_center_size(L, 2, center, size);
    TerrainGrid grid; std::string error;
    if (!terrain_load(*terrain, grid, error)) luaL_error(L, "%s", error.c_str());
    int ox = static_cast<int>(std::floor((center[0]-size[0]/2)/kVoxelSize));
    int oy = static_cast<int>(std::floor((center[1]-size[1]/2)/kVoxelSize));
    int oz = static_cast<int>(std::floor((center[2]-size[2]/2)/kVoxelSize));
    int nx = std::max(1, static_cast<int>(std::round(size[0]/kVoxelSize)));
    int ny = std::max(1, static_cast<int>(std::round(size[1]/kVoxelSize)));
    int nz = std::max(1, static_cast<int>(std::round(size[2]/kVoxelSize)));
    auto& clip = rt->impl_->terrain_clipboard[terrain];
    clip.clear();
    // Store dimensions in a sentinel via negative key range: encode size
    // by also recording every cell position (empty cells absent).
    for (int x = 0; x < nx; ++x) for (int y = 0; y < ny; ++y) for (int z = 0; z < nz; ++z) {
        auto it = grid.cells.find({ox+x, oy+y, oz+z});
        if (it != grid.cells.end()) clip[{x, y, z}] = it->second;
    }
    // Remember extents in the first negative-x entry (material 0 sentinel).
    clip[{-1, nx, ny}] = {static_cast<uint16_t>(nz), 0};
    return 0;
}
int m_terrain_paste_region(lua_State* L) {
    Runtime* rt = rt_of(L);
    Instance* terrain = check_terrain(L, 1);
    std::array<double, 3> center, size; region_center_size(L, 2, center, size);
    bool paste_empty = false;
    if (lua_isboolean(L, 3)) paste_empty = lua_toboolean(L, 3) != 0;
    if (lua_isboolean(L, 4)) paste_empty = lua_toboolean(L, 4) != 0;
    auto cit = rt->impl_->terrain_clipboard.find(terrain);
    if (cit == rt->impl_->terrain_clipboard.end()) luaL_error(L, "PasteRegion: nothing copied (call CopyRegion first)");
    const auto clip = cit->second;
    TerrainGrid grid; std::string error;
    if (!terrain_load(*terrain, grid, error)) luaL_error(L, "%s", error.c_str());
    int ox = static_cast<int>(std::floor((center[0]-size[0]/2)/kVoxelSize));
    int oy = static_cast<int>(std::floor((center[1]-size[1]/2)/kVoxelSize));
    int oz = static_cast<int>(std::floor((center[2]-size[2]/2)/kVoxelSize));
    int nx = std::max(1, static_cast<int>(std::round(size[0]/kVoxelSize)));
    int ny = std::max(1, static_cast<int>(std::round(size[1]/kVoxelSize)));
    int nz = std::max(1, static_cast<int>(std::round(size[2]/kVoxelSize)));
    for (int x = 0; x < nx; ++x) for (int y = 0; y < ny; ++y) for (int z = 0; z < nz; ++z) {
        auto it = clip.find({x, y, z});
        auto key = std::make_tuple(ox+x, oy+y, oz+z);
        if (it != clip.end()) grid.cells[key] = it->second;
        else if (paste_empty) grid.cells.erase(key);
    }
    if (!terrain_store(rt->model(), *terrain, grid, error)) luaL_error(L, "%s", error.c_str());
    return 0;
}

// ------------------------------------------------------------ TextService --

int m_text_service_get_text_size(lua_State* L) {
    // (service, text, textSize, font, frameSize)
    const char* raw = luaL_checkstring(L, 2);
    double size = luaL_checknumber(L, 3);
    UData* frame = check_box(L, 5, UKind::Vector2, "Vector2");
    std::string text(raw);
    // Same DejaVu-backed measurement as the GUI engine (documented
    // substitution); wrap greedily by words like the layout pass.
    std::string font;  // empty = UI default (DejaVu)
    float width = 0;
    int lines = 1;
    float current = 0;
    size_t at = 0;
    auto word_w = [&](const std::string& w) { return measure_text(w, static_cast<float>(size), font); };
    std::string space = " ";
    float space_w = word_w(space);
    while (at <= text.size()) {
        size_t nl = text.find('\n', at);
        std::string line = text.substr(at, nl == std::string::npos ? nl : nl - at);
        // Measure this hard line with wrapping.
        float line_max = 0;
        current = 0;
        size_t wpos = 0;
        bool first_word = true;
        while (wpos <= line.size()) {
            size_t sp = line.find(' ', wpos);
            std::string word = line.substr(wpos, sp == std::string::npos ? sp : sp - wpos);
            float ww = word_w(word);
            float add = ww + (first_word ? 0 : space_w);
            if (current + add > static_cast<float>(frame->n[0]) && current > 0) {
                line_max = std::max(line_max, current);
                lines++;
                current = ww;
            } else {
                current += add;
            }
            first_word = false;
            if (sp == std::string::npos) break;
            wpos = sp + 1;
        }
        line_max = std::max(line_max, current);
        width = std::max(width, line_max);
        if (nl == std::string::npos) break;
        lines++;
        at = nl + 1;
        if (at > text.size()) break;
    }
    // No wrapping when the frame is wider than the text: height is one
    // line (Roblox adds line-height padding; 1.4x approximates it).
    push_vector2(L, width, lines * size * 1.4);
    return 1;
}

int fn_typeof(lua_State* L) {
    if (lua_isuserdata(L, 1)) {
        if (lua_getmetatable(L, 1)) {
            lua_getfield(L, -1, "__type");
            if (lua_isstring(L, -1)) {
                std::string out = lua_tostring(L, -1);
                lua_pop(L, 2);
                lua_pushstring(L, out.c_str());
                return 1;
            }
            lua_pop(L, 2);
        }
        lua_pushstring(L, "userdata");
        return 1;
    }
    lua_pushstring(L, lua_typename(L, lua_type(L, 1)));
    return 1;
}

void log_args(lua_State* L, const char* level) {
    Runtime* rt = rt_of(L);
    std::string message;
    int n = lua_gettop(L);
    for (int i = 1; i <= n; ++i) {
        size_t len = 0;
        const char* s = luaL_tolstring(L, i, &len);
        if (i > 1) message += " ";
        message.append(s ? s : "?", len);
        lua_pop(L, 1);
    }
    rt->log(level, message, rt->impl_->current_owner);
}
int fn_print(lua_State* L) { log_args(L, "print"); return 0; }
int fn_warn(lua_State* L) { log_args(L, "warn"); return 0; }
int fn_tick(lua_State* L) { lua_pushnumber(L, rt_of(L)->time()); return 1; }

// ---------------------------------------------------------------- datatypes --

int v3_index(lua_State* L) {
    auto* u = check_box(L, 1, UKind::Vector3, "Vector3");
    std::string key = luaL_checkstring(L, 2);
    if (key == "X") { lua_pushnumber(L, u->n[0]); return 1; }
    if (key == "Y") { lua_pushnumber(L, u->n[1]); return 1; }
    if (key == "Z") { lua_pushnumber(L, u->n[2]); return 1; }
    if (key == "Magnitude") {
        lua_pushnumber(L, std::sqrt(u->n[0] * u->n[0] + u->n[1] * u->n[1] + u->n[2] * u->n[2]));
        return 1;
    }
    if (key == "Unit") {
        double m = std::sqrt(u->n[0] * u->n[0] + u->n[1] * u->n[1] + u->n[2] * u->n[2]);
        if (m < 1e-9) push_vector3(L, 0, 0, 0);
        else push_vector3(L, u->n[0] / m, u->n[1] / m, u->n[2] / m);
        return 1;
    }
    lua_getfield(L, LUA_REGISTRYINDEX, "blox_vector3_methods");
    lua_pushstring(L, key.c_str());
    lua_rawget(L, -2);
    lua_remove(L, -2);
    return 1;
}
int v3_lerp(lua_State* L) {
    auto* a = check_box(L, 1, UKind::Vector3, "Vector3");
    auto* b = check_box(L, 2, UKind::Vector3, "Vector3");
    double t = luaL_checknumber(L, 3);
    push_vector3(L, a->n[0] + (b->n[0] - a->n[0]) * t, a->n[1] + (b->n[1] - a->n[1]) * t,
                 a->n[2] + (b->n[2] - a->n[2]) * t);
    return 1;
}
int v3_dot(lua_State* L) {
    auto* a = check_box(L, 1, UKind::Vector3, "Vector3");
    auto* b = check_box(L, 2, UKind::Vector3, "Vector3");
    lua_pushnumber(L, a->n[0] * b->n[0] + a->n[1] * b->n[1] + a->n[2] * b->n[2]);
    return 1;
}
int v3_cross(lua_State* L) {
    auto* a = check_box(L, 1, UKind::Vector3, "Vector3");
    auto* b = check_box(L, 2, UKind::Vector3, "Vector3");
    push_vector3(L, a->n[1] * b->n[2] - a->n[2] * b->n[1], a->n[2] * b->n[0] - a->n[0] * b->n[2],
                 a->n[0] * b->n[1] - a->n[1] * b->n[0]);
    return 1;
}
int v3_add(lua_State* L) {
    auto* a = check_box(L, 1, UKind::Vector3, "Vector3");
    auto* b = check_box(L, 2, UKind::Vector3, "Vector3");
    push_vector3(L, a->n[0] + b->n[0], a->n[1] + b->n[1], a->n[2] + b->n[2]);
    return 1;
}
int v3_sub(lua_State* L) {
    auto* a = check_box(L, 1, UKind::Vector3, "Vector3");
    auto* b = check_box(L, 2, UKind::Vector3, "Vector3");
    push_vector3(L, a->n[0] - b->n[0], a->n[1] - b->n[1], a->n[2] - b->n[2]);
    return 1;
}
int v3_mul(lua_State* L) {
    auto* a = check_box(L, 1, UKind::Vector3, "Vector3");
    if (lua_isnumber(L, 2)) {
        double s = lua_tonumber(L, 2);
        push_vector3(L, a->n[0] * s, a->n[1] * s, a->n[2] * s);
    } else {
        auto* b = check_box(L, 2, UKind::Vector3, "Vector3");
        push_vector3(L, a->n[0] * b->n[0], a->n[1] * b->n[1], a->n[2] * b->n[2]);
    }
    return 1;
}
int v3_div(lua_State* L) {
    auto* a = check_box(L, 1, UKind::Vector3, "Vector3");
    if (lua_isnumber(L, 2)) {
        double s = lua_tonumber(L, 2);
        push_vector3(L, a->n[0] / s, a->n[1] / s, a->n[2] / s);
    } else {
        auto* b = check_box(L, 2, UKind::Vector3, "Vector3");
        push_vector3(L, a->n[0] / b->n[0], a->n[1] / b->n[1], a->n[2] / b->n[2]);
    }
    return 1;
}
int v3_unm(lua_State* L) {
    auto* a = check_box(L, 1, UKind::Vector3, "Vector3");
    push_vector3(L, -a->n[0], -a->n[1], -a->n[2]);
    return 1;
}
int v3_eq(lua_State* L) {
    auto* a = check_box(L, 1, UKind::Vector3, "Vector3");
    auto* b = check_box(L, 2, UKind::Vector3, "Vector3");
    lua_pushboolean(L, (a->n[0] == b->n[0] && a->n[1] == b->n[1] && a->n[2] == b->n[2]) ? 1 : 0);
    return 1;
}
int v3_new(lua_State* L) {
    push_vector3(L, luaL_optnumber(L, 1, 0), luaL_optnumber(L, 2, 0), luaL_optnumber(L, 3, 0));
    return 1;
}

using Mat3 = std::array<double, 9>;
Mat3 mat_mul(const Mat3& a, const Mat3& b) {
    Mat3 out{};
    for (int r = 0; r < 3; ++r)
        for (int c = 0; c < 3; ++c)
            out[r * 3 + c] = a[r * 3 + 0] * b[0 * 3 + c] + a[r * 3 + 1] * b[1 * 3 + c] +
                             a[r * 3 + 2] * b[2 * 3 + c];
    return out;
}
std::array<double, 3> mat_vec(const Mat3& m, const std::array<double, 3>& v) {
    return {m[0] * v[0] + m[1] * v[1] + m[2] * v[2], m[3] * v[0] + m[4] * v[1] + m[5] * v[2],
            m[6] * v[0] + m[7] * v[1] + m[8] * v[2]};
}
Mat3 rot_x(double a) { double c = std::cos(a), s = std::sin(a); return {1, 0, 0, 0, c, -s, 0, s, c}; }
Mat3 rot_y(double a) { double c = std::cos(a), s = std::sin(a); return {c, 0, s, 0, 1, 0, -s, 0, c}; }
Mat3 rot_z(double a) { double c = std::cos(a), s = std::sin(a); return {c, -s, 0, s, c, 0, 0, 0, 1}; }

void push_cframe_raw(lua_State* L, const std::array<double, 3>& pos, const Mat3& rot) {
    UData* u = new_box(L, UKind::CFrame, "BloxCFrame");
    u->n = {pos[0], pos[1], pos[2], rot[0], rot[1], rot[2], rot[3], rot[4], rot[5], rot[6], rot[7], rot[8]};
}

int cf_index(lua_State* L) {
    auto* u = check_box(L, 1, UKind::CFrame, "CFrame");
    std::string key = luaL_checkstring(L, 2);
    if (key == "Position") { push_vector3(L, u->n[0], u->n[1], u->n[2]); return 1; }
    if (key == "LookVector") { push_vector3(L, -u->n[5], -u->n[8], -u->n[11]); return 1; }
    if (key == "RightVector") { push_vector3(L, u->n[3], u->n[6], u->n[9]); return 1; }
    if (key == "UpVector") { push_vector3(L, u->n[4], u->n[7], u->n[10]); return 1; }
    if (key == "X") { lua_pushnumber(L, u->n[0]); return 1; }
    if (key == "Y") { lua_pushnumber(L, u->n[1]); return 1; }
    if (key == "Z") { lua_pushnumber(L, u->n[2]); return 1; }
    lua_getfield(L, LUA_REGISTRYINDEX, "blox_cframe_methods");
    lua_pushstring(L, key.c_str());
    lua_rawget(L, -2);
    lua_remove(L, -2);
    return 1;
}
int cf_components(lua_State* L) {
    auto* u = check_box(L, 1, UKind::CFrame, "CFrame");
    for (int i = 0; i < 12; ++i) lua_pushnumber(L, u->n[i]);
    return 12;
}
int cf_mul(lua_State* L) {
    auto* a = check_box(L, 1, UKind::CFrame, "CFrame");
    Mat3 ra{a->n[3], a->n[4], a->n[5], a->n[6], a->n[7], a->n[8], a->n[9], a->n[10], a->n[11]};
    if (lua_isuserdata(L, 2) && static_cast<UData*>(lua_touserdata(L, 2))->kind == UKind::Vector3) {
        auto* b = check_box(L, 2, UKind::Vector3, "Vector3");
        auto r = mat_vec(ra, {b->n[0], b->n[1], b->n[2]});
        push_vector3(L, r[0] + a->n[0], r[1] + a->n[1], r[2] + a->n[2]);
        return 1;
    }
    auto* b = check_box(L, 2, UKind::CFrame, "CFrame");
    Mat3 rb{b->n[3], b->n[4], b->n[5], b->n[6], b->n[7], b->n[8], b->n[9], b->n[10], b->n[11]};
    Mat3 r = mat_mul(ra, rb);
    auto p = mat_vec(ra, {b->n[0], b->n[1], b->n[2]});
    push_cframe_raw(L, {p[0] + a->n[0], p[1] + a->n[1], p[2] + a->n[2]}, r);
    return 1;
}
int cf_new(lua_State* L) {
    int nargs = lua_gettop(L);
    if (nargs == 0) { push_cframe_raw(L, {0, 0, 0}, {1, 0, 0, 0, 1, 0, 0, 0, 1}); return 1; }
    if (nargs == 1 && lua_isuserdata(L, 1)) {
        auto* v = check_box(L, 1, UKind::Vector3, "Vector3");
        push_cframe_raw(L, {v->n[0], v->n[1], v->n[2]}, {1, 0, 0, 0, 1, 0, 0, 0, 1});
        return 1;
    }
    if (nargs == 3) {
        push_cframe_raw(L, {lua_tonumber(L, 1), lua_tonumber(L, 2), lua_tonumber(L, 3)},
                        {1, 0, 0, 0, 1, 0, 0, 0, 1});
        return 1;
    }
    if (nargs == 12) {
        std::array<double, 3> pos{lua_tonumber(L, 1), lua_tonumber(L, 2), lua_tonumber(L, 3)};
        Mat3 rot{};
        for (int i = 0; i < 9; ++i) rot[i] = lua_tonumber(L, 4 + i);
        push_cframe_raw(L, pos, rot);
        return 1;
    }
    luaL_error(L, "unsupported CFrame.new argument count %d", nargs);
    return 0;
}
int cf_look_at(lua_State* L) {
    auto* eye = check_box(L, 1, UKind::Vector3, "Vector3");
    auto* target = check_box(L, 2, UKind::Vector3, "Vector3");
    std::array<double, 3> up{0, 1, 0};
    if (lua_gettop(L) >= 3 && lua_isuserdata(L, 3)) {
        auto* u = check_box(L, 3, UKind::Vector3, "Vector3");
        up = {u->n[0], u->n[1], u->n[2]};
    }
    auto sub = [](std::array<double, 3> a, std::array<double, 3> b) {
        return std::array<double, 3>{a[0] - b[0], a[1] - b[1], a[2] - b[2]};
    };
    auto cross = [](std::array<double, 3> a, std::array<double, 3> b) {
        return std::array<double, 3>{a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2],
                                     a[0] * b[1] - a[1] * b[0]};
    };
    auto norm = [](std::array<double, 3> a) {
        double m = std::sqrt(a[0] * a[0] + a[1] * a[1] + a[2] * a[2]);
        if (m < 1e-9) return std::array<double, 3>{0, 0, 1};
        return std::array<double, 3>{a[0] / m, a[1] / m, a[2] / m};
    };
    auto back = norm(sub({eye->n[0], eye->n[1], eye->n[2]}, {target->n[0], target->n[1], target->n[2]}));
    auto right = norm(cross(up, back));
    if (std::fabs(right[0]) + std::fabs(right[1]) + std::fabs(right[2]) < 1e-6) right = {1, 0, 0};
    auto upv = cross(back, right);
    Mat3 rot{right[0], upv[0], back[0], right[1], upv[1], back[1], right[2], upv[2], back[2]};
    push_cframe_raw(L, {eye->n[0], eye->n[1], eye->n[2]}, rot);
    return 1;
}
int cf_angles(lua_State* L) {
    double rx = luaL_checknumber(L, 1), ry = luaL_checknumber(L, 2), rz = luaL_checknumber(L, 3);
    Mat3 r = mat_mul(rot_y(ry), mat_mul(rot_x(rx), rot_z(rz))); // YXZ order, like Roblox
    push_cframe_raw(L, {0, 0, 0}, r);
    return 1;
}
int cf_eq(lua_State* L) {
    auto* a = check_box(L, 1, UKind::CFrame, "CFrame");
    auto* b = check_box(L, 2, UKind::CFrame, "CFrame");
    lua_pushboolean(L, a->n == b->n ? 1 : 0);
    return 1;
}

int c3_index(lua_State* L) {
    auto* u = check_box(L, 1, UKind::Color3, "Color3");
    std::string key = luaL_checkstring(L, 2);
    if (key == "R") { lua_pushnumber(L, u->n[0]); return 1; }
    if (key == "G") { lua_pushnumber(L, u->n[1]); return 1; }
    if (key == "B") { lua_pushnumber(L, u->n[2]); return 1; }
    lua_pushnil(L);
    return 1;
}
int c3_new(lua_State* L) {
    push_color3(L, luaL_optnumber(L, 1, 0), luaL_optnumber(L, 2, 0), luaL_optnumber(L, 3, 0));
    return 1;
}
int c3_from_rgb(lua_State* L) {
    push_color3(L, luaL_checknumber(L, 1) / 255.0, luaL_checknumber(L, 2) / 255.0,
                luaL_checknumber(L, 3) / 255.0);
    return 1;
}
int c3_eq(lua_State* L) {
    auto* a = check_box(L, 1, UKind::Color3, "Color3");
    auto* b = check_box(L, 2, UKind::Color3, "Color3");
    lua_pushboolean(L, (a->n[0] == b->n[0] && a->n[1] == b->n[1] && a->n[2] == b->n[2]) ? 1 : 0);
    return 1;
}

int v2_index(lua_State* L) {
    auto* u = check_box(L, 1, UKind::Vector2, "Vector2");
    std::string key = luaL_checkstring(L, 2);
    if (key == "X") { lua_pushnumber(L, u->n[0]); return 1; }
    if (key == "Y") { lua_pushnumber(L, u->n[1]); return 1; }
    if (key == "Magnitude") { lua_pushnumber(L, std::sqrt(u->n[0] * u->n[0] + u->n[1] * u->n[1])); return 1; }
    lua_pushnil(L);
    return 1;
}
int v2_new(lua_State* L) {
    push_vector2(L, luaL_optnumber(L, 1, 0), luaL_optnumber(L, 2, 0));
    return 1;
}

int udim_index(lua_State* L) {
    auto* u = check_box(L, 1, UKind::UDim, "UDim");
    std::string key = luaL_checkstring(L, 2);
    if (key == "Scale") { lua_pushnumber(L, u->n[0]); return 1; }
    if (key == "Offset") { lua_pushnumber(L, u->n[1]); return 1; }
    lua_pushnil(L);
    return 1;
}
int udim_new(lua_State* L) {
    push_udim(L, luaL_optnumber(L, 1, 0), luaL_optnumber(L, 2, 0));
    return 1;
}
int udim2_index(lua_State* L) {
    auto* u = check_box(L, 1, UKind::UDim2, "UDim2");
    std::string key = luaL_checkstring(L, 2);
    if (key == "X") { push_udim(L, u->n[0], u->n[1]); return 1; }
    if (key == "Y") { push_udim(L, u->n[2], u->n[3]); return 1; }
    lua_pushnil(L);
    return 1;
}
int udim2_new(lua_State* L) {
    if (lua_gettop(L) == 2 && lua_isuserdata(L, 1) && lua_isuserdata(L, 2)) {
        auto* x = check_box(L, 1, UKind::UDim, "UDim");
        auto* y = check_box(L, 2, UKind::UDim, "UDim");
        push_udim2(L, x->n[0], x->n[1], y->n[0], y->n[1]);
        return 1;
    }
    push_udim2(L, luaL_optnumber(L, 1, 0), luaL_optnumber(L, 2, 0), luaL_optnumber(L, 3, 0),
               luaL_optnumber(L, 4, 0));
    return 1;
}
int udim2_from_scale(lua_State* L) {
    push_udim2(L, luaL_checknumber(L, 1), 0, luaL_checknumber(L, 2), 0);
    return 1;
}
int udim2_from_offset(lua_State* L) {
    push_udim2(L, 0, luaL_checknumber(L, 1), 0, luaL_checknumber(L, 2));
    return 1;
}

int enumitem_index(lua_State* L) {
    auto* u = check_box(L, 1, UKind::EnumItem, "EnumItem");
    std::string key = luaL_checkstring(L, 2);
    if (key == "Name") { lua_pushstring(L, u->s2.c_str()); return 1; }
    if (key == "Value") { lua_pushnumber(L, u->n[0]); return 1; }
    if (key == "EnumType") { lua_pushstring(L, u->s1.c_str()); return 1; }
    lua_pushnil(L);
    return 1;
}
int enumitem_eq(lua_State* L) {
    auto* a = static_cast<UData*>(lua_touserdata(L, 1));
    auto* b = static_cast<UData*>(lua_touserdata(L, 2));
    lua_pushboolean(L, (a && b && a->kind == UKind::EnumItem && b->kind == UKind::EnumItem &&
                        a->s1 == b->s1 && a->n[0] == b->n[0]) ? 1 : 0);
    return 1;
}

} // namespace blox
