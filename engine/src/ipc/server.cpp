// IPC framing and dispatch: newline-delimited JSON-RPC 2.0 on
// stdin/stdout. Domain work lives in the handlers_*.cpp files; this TU
// owns only protocol state (initialize gate, DataModel presence gate)
// and the order in which method families are consulted, which matches
// the pre-refactor server exactly.
#include "blox/ipc/server.hpp"
#include "server_internal.hpp"

#include <algorithm>
#include <cmath>
#include <iostream>

namespace blox::ipc {
namespace {
Json make_error(const Json& id, int code, const std::string& message) {
    Json resp = Json::object();
    resp.set("jsonrpc", Json("2.0"));
    resp.set("id", id);
    Json err = Json::object();
    err.set("code", Json(code));
    err.set("message", Json(message));
    resp.set("error", std::move(err));
    return resp;
}
Json make_result(const Json& id, Json result) {
    Json resp = Json::object();
    resp.set("jsonrpc", Json("2.0"));
    resp.set("id", id);
    resp.set("result", std::move(result));
    return resp;
}
} // namespace

bool require_string(const Json& params, const char* key, std::string& out, std::string& error) {
    const Json* v = params.find(key);
    if (!v || !v->is_string()) { error = std::string("missing string parameter '") + key + "'"; return false; }
    out = v->as_string();
    return true;
}

bool require_vec3(const Json& params, const char* key, std::array<double, 3>& out, std::string& error) {
    const Json* v = params.find(key);
    if (!v || !v->is_array() || v->as_array().size() != 3) {
        error = std::string("parameter '") + key + "' must be [x, y, z]";
        return false;
    }
    for (size_t i = 0; i < 3; ++i) out[i] = v->as_array()[i].as_double();
    return true;
}

bool json_vec3(const Json& params, const char* key, std::array<float, 3>& out) {
    const Json* v = params.find(key);
    if (!v || !v->is_array() || v->as_array().size() != 3) return false;
    for (size_t i = 0; i < 3; ++i) out[i] = static_cast<float>(v->as_array()[i].as_double());
    return true;
}

Json vec3_json(const std::array<float, 3>& v) {
    Json a = Json::array();
    a.push(Json(static_cast<double>(v[0])));
    a.push(Json(static_cast<double>(v[1])));
    a.push(Json(static_cast<double>(v[2])));
    return a;
}

Json camera_json(const blox::CameraInfo& cam) {
    Json obj = Json::object();
    obj.set("position", vec3_json(cam.position));
    obj.set("lookAt", vec3_json(cam.look_at));
    obj.set("up", vec3_json(cam.up));
    obj.set("fovDegrees", Json(static_cast<double>(cam.fov_degrees)));
    obj.set("nearPlane", Json(static_cast<double>(cam.near_plane)));
    obj.set("farPlane", Json(static_cast<double>(cam.far_plane)));
    obj.set("orthographic", Json(cam.orthographic));
    obj.set("orthoHeight", Json(static_cast<double>(cam.ortho_height)));
    return obj;
}

// World-space bounds of a built scene (for camera framing).
void scene_bounds(const blox::Scene& scene, std::array<float, 3>& bmin, std::array<float, 3>& bmax) {
    bmin = {1e30f, 1e30f, 1e30f};
    bmax = {-1e30f, -1e30f, -1e30f};
    for (const auto& draw : scene.draws) {
        const auto& m = draw.model;
        for (const auto& p : draw.positions) {
            float x = m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12];
            float y = m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13];
            float z = m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14];
            bmin = {std::min(bmin[0], x), std::min(bmin[1], y), std::min(bmin[2], z)};
            bmax = {std::max(bmax[0], x), std::max(bmax[1], y), std::max(bmax[2], z)};
        }
    }
    if (bmin[0] > bmax[0]) { bmin = {-10, -5, -10}; bmax = {10, 10, 10}; }
}

blox::CameraInfo frame_camera(const std::array<float, 3>& bmin, const std::array<float, 3>& bmax,
                              const std::string& preset) {
    std::array<float, 3> centre{(bmin[0] + bmax[0]) / 2, (bmin[1] + bmax[1]) / 2, (bmin[2] + bmax[2]) / 2};
    float radius = 0.5f * std::sqrt((bmax[0] - bmin[0]) * (bmax[0] - bmin[0]) +
                                    (bmax[1] - bmin[1]) * (bmax[1] - bmin[1]) +
                                    (bmax[2] - bmin[2]) * (bmax[2] - bmin[2]));
    radius = std::max(radius, 2.0f);
    blox::CameraInfo cam;
    cam.look_at = centre;
    float dist = radius * 2.6f;
    if (preset == "isometric") {
        cam.orthographic = true;
        cam.ortho_height = radius * 1.9f;
        float d = dist;
        cam.position = {centre[0] + d * 0.577f, centre[1] + d * 0.577f, centre[2] + d * 0.577f};
    } else if (preset == "front") {
        cam.position = {centre[0], centre[1], centre[2] + dist};
    } else if (preset == "top") {
        cam.position = {centre[0], centre[1] + dist, centre[2] + 0.001f};
    } else if (preset == "side") {
        cam.position = {centre[0] + dist, centre[1], centre[2]};
    } else { // default three-quarter view
        cam.position = {centre[0] + dist * 0.62f, centre[1] + dist * 0.5f, centre[2] + dist * 0.62f};
    }
    cam.far_plane = std::max(1000.0f, dist * 6);
    return cam;
}

Json brief_instance(DataModel& model, Instance& inst) {
    Json item = Json::object();
    item.set("path", Json(model.path_of(inst)));
    item.set("referent", Json(static_cast<int64_t>(inst.referent)));
    item.set("className", Json(inst.class_name));
    item.set("name", Json(inst.name));
    return item;
}

Json EngineServer::runtime_state() {
            Json state = Json::object();
            state.set("active", Json(runtime_active_));
            if (runtime_) {
                state.set("time", Json(runtime_->time()));
                {
                    Json tweens = Json::array();
                    for (const auto& t : runtime_->tween_states()) {
                        Json item = Json::object();
                        item.set("target", Json(t.target_path));
                        Json props = Json::array();
                        for (const auto& name : t.properties) props.push(Json(name));
                        item.set("properties", std::move(props));
                        item.set("playbackState", Json(t.playback_state));
                        item.set("elapsed", Json(t.elapsed));
                        item.set("duration", Json(t.duration));
                        tweens.push(std::move(item));
                    }
                    state.set("tweens", std::move(tweens));
                }
                state.set("threads", Json(static_cast<int64_t>(runtime_->thread_count())));
                Json logs = Json::array();
                for (const auto& entry : runtime_->logs()) {
                    Json item = Json::object();
                    item.set("level", Json(entry.level));
                    item.set("message", Json(entry.message));
                    item.set("source", Json(entry.source));
                    logs.push(std::move(item));
                }
                state.set("logs", std::move(logs));
                Json errors = Json::array();
                for (const auto& entry : runtime_->errors()) {
                    Json item = Json::object();
                    item.set("message", Json(entry.message));
                    item.set("traceback", Json(entry.traceback));
                    item.set("source", Json(entry.source));
                    errors.push(std::move(item));
                }
                state.set("errors", std::move(errors));
            }
            Json calls = Json::array();
            for (auto& c : outgoing_remotes_) calls.push(std::move(c));
            outgoing_remotes_.clear();
            state.set("remoteCalls", std::move(calls));
            return state;
}

Json EngineServer::character_json() {
                Json c = Json::object();
                if (physics_) {
                    blox::PhysicsWorld::CharacterInfo info = physics_->character_info();
                    c.set("present", Json(info.present));
                    Json pos = Json::array();
                    for (double d : info.position) pos.push(Json(d));
                    c.set("position", std::move(pos));
                    Json vel = Json::array();
                    for (double d : info.velocity) vel.push(Json(d));
                    c.set("velocity", std::move(vel));
                    c.set("grounded", Json(info.grounded));
                    c.set("health", Json(info.health));
                    c.set("state", Json(info.state));
                    c.set("yaw", Json(info.yaw));
                }
                return c;
}

EngineServer::Reply EngineServer::handle(const std::string& method, const Json& params) {
    if (method == "shutdown") { Reply r = Reply::ok(Json(true)); r.shutdown = true; return r; }
    if (method == "initialize") {
        std::string requested;
        if (const Json* v = params.find("protocolVersion")) requested = v->is_string() ? v->as_string() : std::to_string(v->as_int());
        if (requested != kProtocolVersion) {
            return Reply::error(-32000, "protocol version mismatch: engine speaks '" + std::string(kProtocolVersion) + "', client requested '" + requested + "'");
        }
        initialized_ = true;
        Json result = Json::object();
        result.set("protocolVersion", Json(kProtocolVersion));
        result.set("engineVersion", Json(kEngineVersion));
        result.set("implementation", Json("blox-engine (C++20)"));
        return Reply::ok(std::move(result));
    }
    if (!initialized_) return Reply::error(-32001, "not initialized: call 'initialize' first");
    if (method == "ping") return Reply::ok(Json("pong"));
    if (method == "script/analyze" || method == "parity/imagediff") return handle_script(method, params);
    if (method == "datamodel/new" || method == "datamodel/load") return handle_datamodel_newload(method, params);
    if (!has_model_) return Reply::error(-32003, "no DataModel loaded: call 'datamodel/new' or 'datamodel/load' first");
    if (method.rfind("runtime/", 0) == 0) return handle_runtime(method, params);
    if (method.rfind("physics/", 0) == 0) return handle_physics(method, params);
    if (method.rfind("debug/", 0) == 0) return handle_debug(method, params);
    if (method.rfind("terrain/", 0) == 0) return handle_terrain(method, params);
    if (method == "datamodel/snapshot" || method.rfind("instance/", 0) == 0) return handle_datamodel(method, params);
    return handle_render(method, params);
}

int EngineServer::run() {
    std::string line;
    while (std::getline(std::cin, line)) {
        if (line.empty()) continue;
        Json request;
        std::string parse_error;
        if (!Json::parse(line, request, parse_error)) {
            std::cout << make_error(Json(), -32700, "parse error: " + parse_error).dump() << "\n" << std::flush;
            continue;
        }
        const Json* id_j = request.find("id");
        Json id = id_j ? *id_j : Json();
        const Json* method_j = request.find("method");
        if (!method_j || !method_j->is_string()) {
            std::cout << make_error(id, -32600, "missing 'method'").dump() << "\n" << std::flush;
            continue;
        }
        std::string method = method_j->as_string();
        Json params = Json::object();
        if (const Json* p = request.find("params")) params = *p;
        Reply reply = handle(method, params);
        if (reply.is_error) {
            std::cout << make_error(id, reply.code, reply.message).dump() << "\n" << std::flush;
        } else {
            std::cout << make_result(id, std::move(reply.result)).dump() << "\n" << std::flush;
        }
        if (reply.shutdown) break;
    }
    return 0;
}

} // namespace blox::ipc
