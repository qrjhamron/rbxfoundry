// blox-engine: persistent C++ DataModel process.
// Speaks versioned newline-delimited JSON-RPC 2.0 on stdin/stdout.
// Protocol version "1". Diagnostics go to stderr only.
#include <algorithm>
#include <cmath>
#include <iostream>
#include <string>

#include "blox/datamodel.hpp"
#include "blox/json.hpp"
#include "blox/renderer.hpp"
#include "blox/script_tools.hpp"
#include "blox/scene.hpp"
#include "blox/physics.hpp"
#include "blox/terrain.hpp"
#include "blox/runtime.hpp"
#include "blox/ui.hpp"

namespace {

constexpr const char* kProtocolVersion = "1";
constexpr const char* kEngineVersion = "0.2.0-beta.1";

using blox::DataModel;
using blox::Instance;
using blox::Json;
using blox::Variant;

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

} // namespace

int main(int argc, char** argv) {
    for (int i = 1; i < argc; ++i) {
        std::string arg = argv[i];
        if (arg == "--version") { std::cout << "blox-engine " << kEngineVersion << " (protocol " << kProtocolVersion << ")\n"; return 0; }
        if (arg == "--help") {
            std::cout << "blox-engine " << kEngineVersion << "\n"
                      << "Persistent DataModel process. Reads newline-delimited JSON-RPC 2.0\n"
                      << "requests on stdin, writes responses on stdout. Call method\n"
                      << "\"initialize\" with {\"protocolVersion\":\"1\"} first.\n"
                      << "Methods: initialize, ping, datamodel/new, datamodel/load,\n"
                      << "datamodel/snapshot, instance/add, instance/get, instance/set,\n"
                      << "validate, shutdown.\n";
            return 0;
        }
    }

    DataModel model;
    // Runtime (Luau) state: a separate DataModel copy so scripts can
    // never mutate edit-mode state unless the caller commits the
    // runtime snapshot explicitly.
    DataModel runtime_model;
    std::unique_ptr<blox::Runtime> runtime;
    bool runtime_active = false;
    // Phase 9: outgoing RemoteEvent/RemoteFunction calls buffered for the
    // orchestrator (drained into runtime/state, runtime/step results).
    std::vector<Json> outgoing_remotes;
    std::unique_ptr<blox::PhysicsWorld> physics;
    blox::Renderer renderer;
    bool initialized = false;
    bool has_model = false;
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

        auto respond_error = [&](int code, const std::string& msg) {
            std::cout << make_error(id, code, msg).dump() << "\n" << std::flush;
        };
        auto respond = [&](Json result) {
            std::cout << make_result(id, std::move(result)).dump() << "\n" << std::flush;
        };

        if (method == "shutdown") { respond(Json(true)); break; }
        if (method == "initialize") {
            std::string requested;
            if (const Json* v = params.find("protocolVersion")) requested = v->is_string() ? v->as_string() : std::to_string(v->as_int());
            if (requested != kProtocolVersion) {
                respond_error(-32000, "protocol version mismatch: engine speaks '" + std::string(kProtocolVersion) + "', client requested '" + requested + "'");
                continue;
            }
            initialized = true;
            Json result = Json::object();
            result.set("protocolVersion", Json(kProtocolVersion));
            result.set("engineVersion", Json(kEngineVersion));
            result.set("implementation", Json("blox-engine (C++20)"));
            respond(std::move(result));
            continue;
        }
        if (!initialized) { respond_error(-32001, "not initialized: call 'initialize' first"); continue; }
        if (method == "ping") { respond(Json("pong")); continue; }

        if (method == "script/analyze") {
            std::string source, name = "Main";
            if (const Json* v = params.find("source")) source = v->as_string();
            if (const Json* v = params.find("name")) name = v->as_string();
            Json diagnostics = Json::array();
            for (const auto& d : blox::analyze_luau(source, name)) {
                Json item = Json::object();
                item.set("severity", Json(d.severity));
                item.set("kind", Json(d.kind));
                item.set("message", Json(d.message));
                item.set("line", Json(static_cast<int64_t>(d.line)));
                item.set("column", Json(static_cast<int64_t>(d.column)));
                item.set("endLine", Json(static_cast<int64_t>(d.end_line)));
                item.set("endColumn", Json(static_cast<int64_t>(d.end_column)));
                diagnostics.push(std::move(item));
            }
            Json result = Json::object();
            result.set("diagnostics", std::move(diagnostics));
            respond(std::move(result));
            continue;
        }
        if (method == "parity/imagediff") {
            auto str_param = [&](const char* key) -> std::string {
                const Json* v = params.find(key);
                return v && v->is_string() ? v->as_string() : std::string();
            };
            blox::ImageDiff diff;
            std::string diff_error;
            if (!blox::image_diff(str_param("a"), str_param("b"), str_param("heatmap"), diff, diff_error)) {
                respond_error(-32002, diff_error);
                continue;
            }
            Json result = Json::object();
            result.set("width", Json(static_cast<int64_t>(diff.width)));
            result.set("height", Json(static_cast<int64_t>(diff.height)));
            result.set("mae", Json(diff.mae));
            result.set("rmse", Json(diff.rmse));
            result.set("maxDelta", Json(static_cast<int64_t>(diff.max_delta)));
            result.set("diffPixels", Json(static_cast<int64_t>(diff.diff_pixels)));
            const double total = static_cast<double>(diff.width) * diff.height;
            result.set("diffFraction", Json(total > 0 ? diff.diff_pixels / total : 0.0));
            respond(std::move(result));
            continue;
        }

        if (method == "datamodel/new") {
            std::string name = "Place";
            if (const Json* n = params.find("name")) name = n->as_string();
            model.reset(name);
            // Optional offline place metadata (never invented by the
            // engine: callers pass explicit values, defaults are the
            // unpublished zeros Roblox reports for local files).
            {
                std::string ignored;
                if (const Json* v = params.find("placeId"))
                    model.set_property(model.root(), "PlaceId", Variant::make_int64(v->as_int()), ignored);
                if (const Json* v = params.find("gameId"))
                    model.set_property(model.root(), "GameId", Variant::make_string(v->as_string()), ignored);
                if (const Json* v = params.find("placeVersion"))
                    model.set_property(model.root(), "PlaceVersion", Variant::make_int64(v->as_int()), ignored);
                if (const Json* v = params.find("jobId"))
                    model.set_property(model.root(), "JobId", Variant::make_string(v->as_string()), ignored);
            }
            has_model = true;
            Json result = Json::object();
            result.set("tree", model.snapshot());
            respond(std::move(result));
            continue;
        }
        if (method == "datamodel/load") {
            const Json* tree = params.find("tree");
            if (!tree) { respond_error(-32602, "missing 'tree' parameter"); continue; }
            std::string error;
            if (!model.load_snapshot(*tree, error)) { respond_error(-32002, error); continue; }
            has_model = true;
            Json result = Json::object();
            result.set("instanceCount", Json(static_cast<int64_t>(model.instance_count())));
            respond(std::move(result));
            continue;
        }
        if (!has_model) { respond_error(-32003, "no DataModel loaded: call 'datamodel/new' or 'datamodel/load' first"); continue; }

        auto runtime_state = [&]() {
            Json state = Json::object();
            state.set("active", Json(runtime_active));
            if (runtime) {
                state.set("time", Json(runtime->time()));
                state.set("threads", Json(static_cast<int64_t>(runtime->thread_count())));
                Json logs = Json::array();
                for (const auto& entry : runtime->logs()) {
                    Json item = Json::object();
                    item.set("level", Json(entry.level));
                    item.set("message", Json(entry.message));
                    item.set("source", Json(entry.source));
                    logs.push(std::move(item));
                }
                state.set("logs", std::move(logs));
                Json errors = Json::array();
                for (const auto& entry : runtime->errors()) {
                    Json item = Json::object();
                    item.set("message", Json(entry.message));
                    item.set("traceback", Json(entry.traceback));
                    item.set("source", Json(entry.source));
                    errors.push(std::move(item));
                }
                state.set("errors", std::move(errors));
            }
            Json calls = Json::array();
            for (auto& c : outgoing_remotes) calls.push(std::move(c));
            outgoing_remotes.clear();
            state.set("remoteCalls", std::move(calls));
            return state;
        };
        if (method.rfind("runtime/", 0) == 0) {
            if (method == "runtime/begin") {
                // Copy the edit model into a fresh runtime model.
                Json tree = model.snapshot();
                std::string error;
                if (!runtime_model.load_snapshot(tree, error)) { respond_error(-32020, error); continue; }
                runtime = std::make_unique<blox::Runtime>(runtime_model);
                runtime_active = true;
                outgoing_remotes.clear();
                runtime->set_remote_sink([&](const blox::Runtime::RemoteCall& call) {
                    Json item = Json::object();
                    item.set("path", Json(runtime_model.path_of(*call.remote)));
                    item.set("kind", Json(static_cast<int64_t>(call.kind)));
                    item.set("fromPlayer", Json(static_cast<int64_t>(call.from_player)));
                    item.set("invokeId", Json(static_cast<int64_t>(call.invoke_id)));
                    Json args;
                    std::string parse_error;
                    if (Json::parse(call.args_json, args, parse_error)) item.set("args", std::move(args));
                    outgoing_remotes.push_back(std::move(item));
                });
                if (const Json* ctx = params.find("netContext")) {
                    std::string c = ctx->as_string();
                    int pidx = -1;
                    if (const Json* pi = params.find("playerIndex")) pidx = static_cast<int>(pi->as_int(-1));
                    if (c == "server") runtime->set_net_context(blox::Runtime::NetContext::Server, pidx);
                    else if (c == "client") {
                        runtime->set_net_context(blox::Runtime::NetContext::Client, pidx);
                        if (const Json* pn = params.find("playerName")) {
                            std::string perr; Instance* players = runtime_model.resolve_path("Players", perr);
                            if (players) {
                                for (const auto& child : players->children)
                                    if (child->name == pn->as_string()) { runtime->set_local_player(child.get()); break; }
                            }
                        }
                    }
                }
                Json result = Json::object();
                result.set("active", Json(true));
                result.set("instances", Json(static_cast<int64_t>(runtime_model.instance_count())));
                respond(std::move(result));
                continue;
            }
            if (method == "runtime/reset") {
                runtime.reset();
                runtime_active = false;
                Json result = Json::object();
                result.set("active", Json(false));
                respond(std::move(result));
                continue;
            }
            if (!runtime_active || !runtime) { respond_error(-32021, "no runtime session: call 'runtime/begin' first"); continue; }
            if (method == "runtime/runScripts") {
                runtime->start_scripts();
                respond(runtime_state());
                continue;
            }
            if (method == "runtime/eval") {
                std::string source;
                std::string error;
                if (!require_string(params, "source", source, error)) { respond_error(-32602, error); continue; }
                std::string name = "eval";
                if (const Json* n = params.find("name")) name = n->as_string();
                std::string error_msg, result_json;
                bool ok = runtime->eval(source, name, error_msg, result_json);
                Json result = runtime_state();
                result.set("ok", Json(ok));
                if (!ok) result.set("error", Json(error_msg));
                Json values;
                std::string parse_error;
                if (Json::parse(result_json.empty() ? "[]" : result_json, values, parse_error))
                    result.set("returns", std::move(values));
                respond(std::move(result));
                continue;
            }
            if (method == "runtime/step") {
                double dt = 1.0 / 60.0;
                if (const Json* d = params.find("dt")) dt = d->as_double(dt);
                int steps = 1;
                if (const Json* n = params.find("steps")) steps = static_cast<int>(n->as_int(1));
                for (int i = 0; i < steps; ++i) runtime->step(dt);
                respond(runtime_state());
                continue;
            }
            if (method == "runtime/state") {
                respond(runtime_state());
                continue;
            }
            if (method == "runtime/click" || method == "runtime/hover") {
                double x = 0, y = 0;
                uint32_t vw = 1280, vh = 720;
                if (const Json* v = params.find("x")) x = v->as_double();
                if (const Json* v = params.find("y")) y = v->as_double();
                if (const Json* v = params.find("width")) vw = static_cast<uint32_t>(v->as_int(vw));
                if (const Json* v = params.find("height")) vh = static_cast<uint32_t>(v->as_int(vh));
                auto hit = method == "runtime/click" ? runtime->gui_click(x, y, vw, vh)
                                                     : runtime->gui_hover(x, y, vw, vh);
                Json result = runtime_state();
                Json hit_json = Json::object();
                hit_json.set("hit", Json(hit.hit));
                hit_json.set("path", Json(hit.path));
                hit_json.set("className", Json(hit.class_name));
                hit_json.set("event", Json(hit.event));
                result.set("hit", std::move(hit_json));
                respond(std::move(result));
                continue;
            }
            if (method == "runtime/typeText") {
                std::string text;
                bool submit = false;
                if (const Json* v = params.find("text")) text = v->as_string();
                if (const Json* v = params.find("submit")) submit = v->as_bool();
                Json result = runtime_state();
                result.set("typed", Json(runtime->gui_type_text(text, submit)));
                respond(std::move(result));
                continue;
            }
            if (method == "runtime/keys") {
                // UserInputService key state (play mode input feed).
                if (const Json* keys = params.find("keys")) {
                    if (keys->is_object()) {
                        for (const auto& [name, down] : keys->as_object())
                            runtime->set_key_state(name, down.as_bool());
                    }
                }
                respond(runtime_state());
                continue;
            }
            if (method == "runtime/snapshot") {
                Json result = Json::object();
                result.set("tree", runtime_model.snapshot());
                respond(std::move(result));
                continue;
            }
            if (method == "runtime/netDeliver") {
                std::string path;
                if (const Json* v = params.find("path")) path = v->as_string();
                std::string rerr; Instance* remote = runtime_model.resolve_path(path, rerr);
                if (!remote) { respond_error(-32023, "no remote instance at path '" + path + "'"); continue; }
                Instance* from_player = nullptr;
                if (const Json* fp = params.find("fromPlayerPath"))
                    { std::string ferr; from_player = runtime_model.resolve_path(fp->as_string(), ferr); }
                std::string args = "[]";
                if (const Json* a = params.find("args")) args = a->dump();
                runtime->deliver_remote(remote, from_player, args);
                respond(runtime_state());
                continue;
            }
            if (method == "runtime/netInvokeHandler") {
                std::string path;
                if (const Json* v = params.find("path")) path = v->as_string();
                std::string rerr; Instance* remote = runtime_model.resolve_path(path, rerr);
                if (!remote) { respond_error(-32023, "no remote instance at path '" + path + "'"); continue; }
                Instance* from_player = nullptr;
                if (const Json* fp = params.find("fromPlayerPath"))
                    { std::string ferr; from_player = runtime_model.resolve_path(fp->as_string(), ferr); }
                std::string args = "[]";
                if (const Json* a = params.find("args")) args = a->dump();
                bool ok = false;
                std::string result = runtime->invoke_remote_handler(remote, from_player, args, ok);
                Json out = Json::object();
                out.set("ok", Json(ok));
                Json values;
                std::string parse_error;
                if (ok && Json::parse(result.empty() ? "[]" : result, values, parse_error))
                    out.set("result", std::move(values));
                respond(std::move(out));
                continue;
            }
            if (method == "runtime/netCompleteInvoke") {
                int invoke_id = 0;
                if (const Json* v = params.find("invokeId")) invoke_id = static_cast<int>(v->as_int());
                bool ok = false;
                if (const Json* v = params.find("ok")) ok = v->as_bool();
                std::string error;
                if (const Json* v = params.find("error")) error = v->as_string();
                std::string result = "[]";
                if (const Json* v = params.find("result")) result = v->dump();
                runtime->complete_invoke(invoke_id, result, ok, error);
                respond(runtime_state());
                continue;
            }
            if (method == "runtime/setProperty") {
                std::string path;
                if (const Json* v = params.find("path")) path = v->as_string();
                std::string ierr;
                Instance* inst = runtime_model.resolve_path(path, ierr);
                if (!inst) { respond_error(-32023, "no instance at path '" + path + "'"); continue; }
                const Json* prop = params.find("property");
                const Json* value = params.find("value");
                if (!prop || !value) { respond_error(-32602, "setProperty needs 'property' and 'value'"); continue; }
                Variant variant;
                std::string error;
                if (!Variant::from_json(*value, variant, error) ||
                    !runtime_model.set_property(*inst, prop->as_string(), variant, error)) {
                    respond_error(-32024, error);
                    continue;
                }
                respond(runtime_state());
                continue;
            }
            if (method == "runtime/addInstance") {
                std::string parent_path;
                if (const Json* v = params.find("parentPath")) parent_path = v->as_string();
                std::string ierr;
                Instance* parent = runtime_model.resolve_path(parent_path, ierr);
                if (!parent) { respond_error(-32023, "no instance at path '" + parent_path + "'"); continue; }
                std::string klass = "Part";
                if (const Json* v = params.find("class")) klass = v->as_string();
                std::string iname;
                if (const Json* v = params.find("name")) iname = v->as_string();
                Instance& created = runtime_model.add_instance(*parent, klass, iname);
                if (const Json* props = params.find("properties")) {
                    if (props->is_object()) {
                        for (const auto& [key, value] : props->as_object()) {
                            Variant variant;
                            std::string error;
                            if (Variant::from_json(value, variant, error))
                                runtime_model.set_property(created, key, variant, error);
                        }
                    }
                }
                Instance* raw = &created;
                Json result = Json::object();
                result.set("path", Json(runtime_model.path_of(*raw)));
                respond(std::move(result));
                continue;
            }
            if (method == "runtime/removeInstance") {
                std::string path;
                if (const Json* v = params.find("path")) path = v->as_string();
                std::string ierr;
                Instance* inst = runtime_model.resolve_path(path, ierr);
                if (!inst || !inst->parent) { respond_error(-32023, "no removable instance at path '" + path + "'"); continue; }
                Instance* parent = inst->parent;
                parent->children.erase(std::remove_if(parent->children.begin(), parent->children.end(),
                    [&](const std::unique_ptr<Instance>& c) { return c.get() == inst; }), parent->children.end());
                respond(runtime_state());
                continue;
            }
            if (method == "runtime/fireSignal") {
                std::string path;
                if (const Json* v = params.find("path")) path = v->as_string();
                std::string ierr; Instance* inst = runtime_model.resolve_path(path, ierr);
                if (!inst) { respond_error(-32023, "no instance at path '" + path + "'"); continue; }
                std::string signal = "Changed";
                if (const Json* v = params.find("signal")) signal = v->as_string();
                std::string args = "[]";
                if (const Json* a = params.find("args")) args = a->dump();
                runtime->fire_signal_json(inst, signal, args);
                respond(runtime_state());
                continue;
            }
            respond_error(-32601, "unknown runtime method '" + method + "'");
            continue;
        }

        if (method.rfind("physics/", 0) == 0) {
            if (!runtime_active || !runtime) { respond_error(-32021, "no runtime session: call 'runtime/begin' first"); continue; }
            auto character_json = [&]() {
                Json c = Json::object();
                if (physics) {
                    blox::PhysicsWorld::CharacterInfo info = physics->character_info();
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
            };
            if (method == "physics/begin") {
                std::string rig = "R6";
                if (const Json* r = params.find("rig")) rig = r->as_string();
                physics = std::make_unique<blox::PhysicsWorld>(runtime_model);
                physics->spawn_character(rig);
                physics->build_world();
                runtime->set_physics(physics.get());
                Json result = Json::object();
                result.set("bodies", Json(static_cast<int64_t>(physics->body_count())));
                result.set("character", character_json());
                respond(std::move(result));
                continue;
            }
            if (method == "physics/beginMulti") {
                physics = std::make_unique<blox::PhysicsWorld>(runtime_model);
                runtime->set_physics(physics.get());
                Json chars = Json::array();
                if (const Json* players = params.find("players")) {
                    if (players->is_array()) {
                        for (const Json& pl : players->as_array()) {
                            std::string pname = pl.find("name") ? pl.find("name")->as_string() : "Player";
                            std::string rig = pl.find("rig") ? pl.find("rig")->as_string() : "R6";
                            int idx = physics->spawn_character_multi(rig, pname);
                            Json item = Json::object();
                            item.set("index", Json(static_cast<int64_t>(idx)));
                            item.set("name", Json(pname));
                            chars.push(std::move(item));
                        }
                    }
                }
                physics->build_world();
                runtime->set_character_resolver([&](Instance* player) -> Instance* {
                    if (!physics || !player) return nullptr;
                    for (size_t i = 0; i < physics->character_count(); ++i) {
                        Instance* m = physics->character_model_at(i);
                        if (m && m->name == player->name) return m;
                    }
                    return nullptr;
                });
                Json result = Json::object();
                result.set("bodies", Json(static_cast<int64_t>(physics->body_count())));
                result.set("characters", std::move(chars));
                respond(std::move(result));
                continue;
            }
            if (!physics) { respond_error(-32022, "physics not begun: call 'physics/begin' first"); continue; }
            if (method == "physics/step") {
                double dt = 1.0 / 60.0;
                if (const Json* d = params.find("dt")) dt = d->as_double(dt);
                int steps = 1;
                if (const Json* n = params.find("steps")) steps = static_cast<int>(n->as_int(1));
                std::array<double, 2> move{0, 0};
                bool jump = false;
                double yaw = 0.0;
                if (const Json* input = params.find("input")) {
                    if (const Json* m = input->find("move")) {
                        if (m->is_array() && m->as_array().size() == 2) {
                            move = {m->as_array()[0].as_double(), m->as_array()[1].as_double()};
                        }
                    }
                    if (const Json* j = input->find("jump")) jump = j->as_bool();
                    if (const Json* y = input->find("yaw")) yaw = y->as_double();
                }
                Json events = Json::array();
                for (int i = 0; i < steps; ++i) {
                    physics->set_character_input(move, jump, yaw);
                    std::vector<Instance*> moved = physics->step(dt);
                    runtime->notify_moved(moved);
                    runtime->step(dt);
                    physics->set_character_anim_driven(
                        0, runtime->animation_playing_in(physics->character_model_at(0)));
                    for (auto& [touched, other] : physics->take_touch_events()) {
                        Json args = Json::array();
                        Json tag = Json::object();
                        tag.set("$t", Json("Instance"));
                        tag.set("p", Json(runtime_model.path_of(*other)));
                        args.push(std::move(tag));
                        runtime->fire_signal_json(touched, "Touched", args.dump());
                    }
                    for (const std::string& event : physics->take_events()) {
                        events.push(Json(event));
                        if (event == "died" && physics->humanoid())
                            runtime->fire_event(physics->humanoid(), "Died");
                    }
                    if (physics->humanoid())
                        runtime->set_humanoid_state(physics->humanoid(),
                                                    physics->character_info().state);
                }
                Json result = Json::object();
                result.set("character", character_json());
                result.set("time", Json(physics->time()));
                result.set("events", std::move(events));
                result.set("threads", Json(static_cast<int64_t>(runtime->thread_count())));
                Json logs = Json::array();
                for (const auto& entry : runtime->logs()) {
                    Json item = Json::object();
                    item.set("level", Json(entry.level));
                    item.set("message", Json(entry.message));
                    item.set("source", Json(entry.source));
                    logs.push(std::move(item));
                }
                result.set("logs", std::move(logs));
                Json errors = Json::array();
                for (const auto& entry : runtime->errors()) {
                    Json item = Json::object();
                    item.set("message", Json(entry.message));
                    item.set("traceback", Json(entry.traceback));
                    item.set("source", Json(entry.source));
                    errors.push(std::move(item));
                }
                result.set("errors", std::move(errors));
                respond(std::move(result));
                continue;
            }
            if (method == "physics/state") {
                Json result = Json::object();
                result.set("character", character_json());
                result.set("time", Json(physics->time()));
                result.set("bodies", Json(static_cast<int64_t>(physics->body_count())));
                respond(std::move(result));
                continue;
            }
            if (method == "physics/raycast") {
                auto vec3_param = [&](const char* key, std::array<double, 3>& out) {
                    const Json* v = params.find(key);
                    if (!v || !v->is_array() || v->as_array().size() != 3) return false;
                    out = {v->as_array()[0].as_double(), v->as_array()[1].as_double(), v->as_array()[2].as_double()};
                    return true;
                };
                std::array<double, 3> origin{}, direction{};
                if (!vec3_param("origin", origin) || !vec3_param("direction", direction)) {
                    respond_error(-32602, "raycast needs origin and direction [x,y,z]");
                    continue;
                }
                double max_distance = 1000.0;
                if (const Json* m = params.find("maxDistance")) max_distance = m->as_double(max_distance);
                blox::PhysicsWorld::RayHit hit = physics->raycast(origin, direction, max_distance);
                Json result = Json::object();
                result.set("hit", Json(hit.hit));
                if (hit.hit) {
                    result.set("path", Json(hit.path));
                    Json point = Json::array();
                    for (double d : hit.point) point.push(Json(d));
                    result.set("point", std::move(point));
                    Json normal = Json::array();
                    for (double d : hit.normal) normal.push(Json(d));
                    result.set("normal", std::move(normal));
                    result.set("distance", Json(hit.distance));
                }
                respond(std::move(result));
                continue;
            }
            if (method == "physics/stepMulti") {
                if (!physics) { respond_error(-32022, "physics not begun: call 'physics/beginMulti' first"); continue; }
                double dt = 1.0 / 60.0;
                if (const Json* d = params.find("dt")) dt = d->as_double(dt);
                int steps = 1;
                if (const Json* n = params.find("steps")) steps = static_cast<int>(n->as_int(1));
                const Json* inputs = params.find("inputs");
                for (int i = 0; i < steps; ++i) {
                    if (inputs && inputs->is_array()) {
                        size_t c = 0;
                        for (const Json& input : inputs->as_array()) {
                            std::array<double, 2> move{0, 0};
                            bool jump = false;
                            double yaw = 0.0;
                            if (const Json* m = input.find("move")) {
                                if (m->is_array() && m->as_array().size() == 2)
                                    move = {m->as_array()[0].as_double(), m->as_array()[1].as_double()};
                            }
                            if (const Json* j = input.find("jump")) jump = j->as_bool();
                            if (const Json* y = input.find("yaw")) yaw = y->as_double();
                            physics->set_character_input_at(c, move, jump, yaw);
                            ++c;
                        }
                    }
                    std::vector<Instance*> moved = physics->step(dt);
                    runtime->notify_moved(moved);
                    runtime->step(dt);
                    for (size_t c = 0; c < physics->character_count(); ++c)
                        physics->set_character_anim_driven(
                            c, runtime->animation_playing_in(physics->character_model_at(c)));
                    for (size_t c = 0; c < physics->character_count(); ++c) {
                        for (const std::string& event : physics->take_events_at(c)) {
                            if (event == "died" && physics->humanoid_at(c))
                                runtime->fire_event(physics->humanoid_at(c), "Died");
                        }
                        if (physics->humanoid_at(c))
                            runtime->set_humanoid_state(physics->humanoid_at(c),
                                                        physics->character_info_at(c).state);
                    }
                    for (auto& [touched, other] : physics->take_touch_events()) {
                        Json args = Json::array();
                        Json tag = Json::object();
                        tag.set("$t", Json("Instance"));
                        tag.set("p", Json(runtime_model.path_of(*other)));
                        args.push(std::move(tag));
                        runtime->fire_signal_json(touched, "Touched", args.dump());
                    }
                }
                Json result = Json::object();
                Json chars = Json::array();
                for (size_t c = 0; c < physics->character_count(); ++c) {
                    blox::PhysicsWorld::CharacterInfo info = physics->character_info_at(c);
                    Json item = Json::object();
                    item.set("present", Json(info.present));
                    Json pos = Json::array();
                    for (double d : info.position) pos.push(Json(d));
                    item.set("position", std::move(pos));
                    Json vel = Json::array();
                    for (double d : info.velocity) vel.push(Json(d));
                    item.set("velocity", std::move(vel));
                    item.set("grounded", Json(info.grounded));
                    item.set("health", Json(info.health));
                    item.set("state", Json(info.state));
                    item.set("yaw", Json(info.yaw));
                    if (Instance* m = physics->character_model_at(c)) item.set("name", Json(m->name));
                    chars.push(std::move(item));
                }
                result.set("characters", std::move(chars));
                result.set("time", Json(physics->time()));
                Json logs = Json::array();
                for (const auto& entry : runtime->logs()) {
                    Json item = Json::object();
                    item.set("level", Json(entry.level));
                    item.set("message", Json(entry.message));
                    item.set("source", Json(entry.source));
                    logs.push(std::move(item));
                }
                result.set("logs", std::move(logs));
                Json errors = Json::array();
                for (const auto& entry : runtime->errors()) {
                    Json item = Json::object();
                    item.set("message", Json(entry.message));
                    item.set("traceback", Json(entry.traceback));
                    item.set("source", Json(entry.source));
                    errors.push(std::move(item));
                }
                result.set("errors", std::move(errors));
                Json calls = Json::array();
                for (auto& c : outgoing_remotes) calls.push(std::move(c));
                outgoing_remotes.clear();
                result.set("remoteCalls", std::move(calls));
                respond(std::move(result));
                continue;
            }
            if (method == "physics/queryBox" || method == "physics/queryRadius") {
                auto vec3_param = [&](const char* key, std::array<double, 3>& out) {
                    const Json* v = params.find(key);
                    if (!v || !v->is_array() || v->as_array().size() != 3) return false;
                    out = {v->as_array()[0].as_double(), v->as_array()[1].as_double(), v->as_array()[2].as_double()};
                    return true;
                };
                std::array<double, 3> center{};
                if (!vec3_param("center", center)) {
                    respond_error(-32602, "query needs center [x,y,z]");
                    continue;
                }
                std::vector<Instance*> hits;
                if (method == "physics/queryBox") {
                    std::array<double, 3> size{1, 1, 1};
                    if (!vec3_param("size", size)) { respond_error(-32602, "queryBox needs size [x,y,z]"); continue; }
                    hits = physics->parts_in_box(center, size);
                } else {
                    double radius = 5.0;
                    if (const Json* r = params.find("radius")) radius = r->as_double(radius);
                    hits = physics->parts_in_radius(center, radius);
                }
                Json result = Json::object();
                Json paths = Json::array();
                for (Instance* inst : hits) paths.push(Json(runtime_model.path_of(*inst)));
                result.set("paths", std::move(paths));
                respond(std::move(result));
                continue;
            }
            if (method == "physics/end") {
                physics.reset();
                Json result = Json::object();
                result.set("active", Json(false));
                respond(std::move(result));
                continue;
            }
            respond_error(-32601, "unknown physics method '" + method + "'");
            continue;
        }

        if (method.rfind("debug/", 0) == 0) {
            auto debug_json = [&](int frame, bool with_vars) {
                std::vector<blox::Runtime::DebugVariable> vars;
                auto status = runtime->debug_status(frame, with_vars ? &vars : nullptr);
                Json dbg = Json::object();
                dbg.set("paused", Json(status.paused));
                dbg.set("finished", Json(status.finished));
                dbg.set("reason", Json(status.reason));
                dbg.set("script", Json(status.script));
                dbg.set("line", Json(static_cast<int64_t>(status.line)));
                Json stack = Json::array();
                for (const auto& f : status.stack) {
                    Json item = Json::object();
                    item.set("function", Json(f.function));
                    item.set("source", Json(f.source));
                    item.set("line", Json(static_cast<int64_t>(f.line)));
                    stack.push(std::move(item));
                }
                dbg.set("stack", std::move(stack));
                Json variables = Json::array();
                for (const auto& v : vars) {
                    Json item = Json::object();
                    item.set("name", Json(v.name));
                    item.set("kind", Json(v.kind));
                    item.set("value", Json(v.value));
                    variables.push(std::move(item));
                }
                dbg.set("variables", std::move(variables));
                Json result = runtime_state();
                result.set("debug", std::move(dbg));
                return result;
            };
            if (method == "debug/begin") {
                std::map<std::string, std::vector<int>> bps;
                if (const Json* arr = params.find("breakpoints")) {
                    if (arr->is_array()) {
                        for (const auto& item : arr->as_array()) {
                            std::string script;
                            int line = 0;
                            if (const Json* sp = item.find("script")) script = sp->as_string();
                            if (const Json* ln = item.find("line")) line = static_cast<int>(ln->as_int());
                            if (!script.empty() && line > 0) bps[script].push_back(line);
                        }
                    }
                }
                runtime->debug_set_breakpoints(bps);
                runtime->start_scripts();
                respond(debug_json(0, true));
                continue;
            }
            if (method == "debug/state") {
                int frame = 0;
                if (const Json* f = params.find("frame")) frame = static_cast<int>(f->as_int());
                respond(debug_json(frame, true));
                continue;
            }
            if (method == "debug/resume") {
                std::string mode = "continue";
                if (const Json* m = params.find("mode")) mode = m->as_string();
                if (mode != "continue" && mode != "stepInto" && mode != "stepOver" && mode != "stepOut") {
                    respond_error(-32602, "debug/resume mode must be continue|stepInto|stepOver|stepOut");
                    continue;
                }
                if (!runtime->debug_resume(mode)) {
                    respond_error(-32026, "no paused thread to resume");
                    continue;
                }
                respond(debug_json(0, true));
                continue;
            }
            respond_error(-32601, "unknown debug method '" + method + "'");
            continue;
        }

        if (method.rfind("terrain/", 0) == 0) {
            std::string error;
            Instance* workspace = nullptr;
            for (const auto& child : model.root().children)
                if (child->class_name == "Workspace") workspace = child.get();
            if (!workspace) { respond_error(-32010, "place has no Workspace"); continue; }
            Instance* terrain = find_terrain(model);
            auto ensure_terrain = [&]() -> Instance* {
                if (terrain) return terrain;
                Instance& created = model.add_instance(*workspace, "Terrain", "Terrain");
                terrain = &created;
                return terrain;
            };
            if (method == "terrain/info") {
                Json result = Json::object();
                result.set("hasTerrain", Json(terrain != nullptr));
                int64_t voxels = 0;
                Json materials = Json::object();
                if (terrain) {
                    blox::TerrainGrid grid;
                    if (!terrain_load(*terrain, grid, error)) { respond_error(-32025, error); continue; }
                    voxels = static_cast<int64_t>(grid.cells.size());
                    std::map<uint16_t, int64_t> counts;
                    for (const auto& [key, value] : grid.cells) counts[value.first]++;
                    for (const auto& [mat, n] : counts)
                        materials.set(std::to_string(mat), Json(n));
                }
                result.set("voxels", Json(voxels));
                result.set("materialsByEnumValue", std::move(materials));
                result.set("voxelSize", Json(blox::kVoxelSize));
                respond(std::move(result));
                continue;
            }
            if (method == "terrain/fill" || method == "terrain/fillBall") {
                auto vec3_param = [&](const char* key, std::array<double, 3>& out) {
                    const Json* v = params.find(key);
                    if (!v || !v->is_array() || v->as_array().size() != 3) return false;
                    out = {v->as_array()[0].as_double(), v->as_array()[1].as_double(), v->as_array()[2].as_double()};
                    return true;
                };
                std::array<double, 3> center{};
                if (!vec3_param("center", center)) { respond_error(-32602, "terrain fill needs center [x,y,z]"); continue; }
                int64_t material = 1280;
                if (const Json* m = params.find("material")) material = m->as_int(material);
                Instance* target = ensure_terrain();
                blox::TerrainGrid grid;
                if (!terrain_load(*target, grid, error)) { respond_error(-32025, error); continue; }
                if (method == "terrain/fill") {
                    std::array<double, 3> size{4, 4, 4};
                    if (!vec3_param("size", size)) { respond_error(-32602, "terrain/fill needs size [x,y,z]"); continue; }
                    terrain_fill_block(grid, center, size, static_cast<uint16_t>(material));
                } else {
                    double radius = 4.0;
                    if (const Json* r = params.find("radius")) radius = r->as_double(radius);
                    terrain_fill_ball(grid, center, radius, static_cast<uint16_t>(material));
                }
                if (!terrain_store(model, *target, grid, error)) { respond_error(-32025, error); continue; }
                Json result = Json::object();
                result.set("voxels", Json(static_cast<int64_t>(grid.cells.size())));
                respond(std::move(result));
                continue;
            }
            if (method == "terrain/flatten" || method == "terrain/smooth" || method == "terrain/paint") {
                auto vec3_param = [&](const char* key, std::array<double, 3>& out) {
                    const Json* v = params.find(key);
                    if (!v || !v->is_array() || v->as_array().size() != 3) return false;
                    out = {v->as_array()[0].as_double(), v->as_array()[1].as_double(), v->as_array()[2].as_double()};
                    return true;
                };
                std::array<double, 3> center{}, size{16, 16, 16};
                if (!vec3_param("center", center)) { respond_error(-32602, "terrain op needs center [x,y,z]"); continue; }
                if (!vec3_param("size", size)) { respond_error(-32602, "terrain op needs size [x,y,z]"); continue; }
                int64_t material = 1280;
                if (const Json* m = params.find("material")) material = m->as_int(material);
                Instance* target = ensure_terrain();
                blox::TerrainGrid grid;
                if (!terrain_load(*target, grid, error)) { respond_error(-32025, error); continue; }
                if (method == "terrain/flatten") {
                    double height = center[1];
                    if (const Json* h = params.find("height")) height = h->as_double(height);
                    terrain_flatten(grid, center, size, height, static_cast<uint16_t>(material));
                } else if (method == "terrain/smooth") {
                    terrain_smooth(grid, center, size);
                } else {
                    terrain_paint(grid, center, size, static_cast<uint16_t>(material));
                }
                if (!terrain_store(model, *target, grid, error)) { respond_error(-32025, error); continue; }
                Json result = Json::object();
                result.set("voxels", Json(static_cast<int64_t>(grid.cells.size())));
                respond(std::move(result));
                continue;
            }
            if (method == "terrain/clear") {
                if (terrain) {
                    blox::TerrainGrid grid;
                    if (!terrain_store(model, *terrain, grid, error)) { respond_error(-32025, error); continue; }
                }
                Json result = Json::object();
                result.set("voxels", Json(static_cast<int64_t>(0)));
                respond(std::move(result));
                continue;
            }
            respond_error(-32601, "unknown terrain method '" + method + "'");
            continue;
        }

        if (method == "datamodel/snapshot") {
            Json result = Json::object();
            result.set("tree", model.snapshot());
            result.set("instanceCount", Json(static_cast<int64_t>(model.instance_count())));
            respond(std::move(result));
        } else if (method == "instance/add") {
            std::string parent_path, class_name, name, error;
            if (!require_string(params, "parent", parent_path, error) ||
                !require_string(params, "className", class_name, error)) { respond_error(-32602, error); continue; }
            if (class_name.empty()) { respond_error(-32602, "className must not be empty"); continue; }
            if (const Json* n = params.find("name")) name = n->as_string();
            Instance* parent = model.resolve_path(parent_path, error);
            if (!parent) { respond_error(-32004, error); continue; }
            Instance& inst = model.add_instance(*parent, class_name, name);
            Json result = Json::object();
            result.set("path", Json(model.path_of(inst)));
            result.set("instance", DataModel::instance_to_json(inst));
            respond(std::move(result));
        } else if (method == "instance/get") {
            std::string target, error;
            if (!require_string(params, "target", target, error)) { respond_error(-32602, error); continue; }
            Instance* inst = model.resolve_path(target, error);
            if (!inst) { respond_error(-32004, error); continue; }
            Json result = Json::object();
            result.set("path", Json(model.path_of(*inst)));
            result.set("instance", DataModel::instance_to_json(*inst));
            respond(std::move(result));
        } else if (method == "instance/set") {
            std::string target, property, error;
            if (!require_string(params, "target", target, error) ||
                !require_string(params, "property", property, error)) { respond_error(-32602, error); continue; }
            const Json* value_j = params.find("value");
            if (!value_j) { respond_error(-32602, "missing 'value' parameter"); continue; }
            Variant value;
            if (!Variant::from_json(*value_j, value, error)) { respond_error(-32602, error); continue; }
            Instance* inst = model.resolve_path(target, error);
            if (!inst) { respond_error(-32004, error); continue; }
            if (!model.set_property(*inst, property, value, error)) { respond_error(-32005, error); continue; }
            Json result = Json::object();
            result.set("path", Json(model.path_of(*inst)));
            result.set("instance", DataModel::instance_to_json(*inst));
            respond(std::move(result));
        } else if (method == "instance/clone") {
            std::string target, error;
            if (!require_string(params, "target", target, error)) { respond_error(-32602, error); continue; }
            Instance* inst = model.resolve_path(target, error);
            if (!inst) { respond_error(-32004, error); continue; }
            Instance* parent = inst->parent;
            std::string parent_path;
            if (require_string(params, "parent", parent_path, error)) {
                parent = model.resolve_path(parent_path, error);
                if (!parent) { respond_error(-32004, error); continue; }
            }
            if (!parent) { respond_error(-32004, "cannot clone the DataModel root"); continue; }
            std::string name;
            if (const Json* n = params.find("name")) name = n->as_string();
            Instance& copy = model.clone_instance(*inst, *parent, name);
            Json result = brief_instance(model, copy);
            result.set("instance", DataModel::instance_to_json(copy));
            respond(std::move(result));
        } else if (method == "instance/delete") {
            std::string target, error;
            if (!require_string(params, "target", target, error)) { respond_error(-32602, error); continue; }
            Instance* inst = model.resolve_path(target, error);
            if (!inst) { respond_error(-32004, error); continue; }
            if (inst == &model.root()) { respond_error(-32004, "cannot delete the DataModel root"); continue; }
            std::string path = model.path_of(*inst);
            size_t removed = model.delete_instance(*inst);
            Json result = Json::object();
            result.set("path", Json(path));
            result.set("deletedCount", Json(static_cast<int64_t>(removed)));
            result.set("instanceCount", Json(static_cast<int64_t>(model.instance_count())));
            respond(std::move(result));
        } else if (method == "instance/reparent") {
            std::string target, parent_path, error;
            if (!require_string(params, "target", target, error) ||
                !require_string(params, "parent", parent_path, error)) { respond_error(-32602, error); continue; }
            Instance* inst = model.resolve_path(target, error);
            if (!inst) { respond_error(-32004, error); continue; }
            Instance* parent = model.resolve_path(parent_path, error);
            if (!parent) { respond_error(-32004, error); continue; }
            if (!model.reparent(*inst, *parent, error)) { respond_error(-32005, error); continue; }
            Json result = brief_instance(model, *inst);
            respond(std::move(result));
        } else if (method == "instance/group") {
            const Json* targets_j = params.find("targets");
            if (!targets_j || !targets_j->is_array() || targets_j->as_array().empty()) {
                respond_error(-32602, "'targets' must be a non-empty array of paths"); continue;
            }
            std::vector<Instance*> targets;
            std::string error;
            bool ok = true;
            for (const auto& t : targets_j->as_array()) {
                Instance* inst = model.resolve_path(t.as_string(), error);
                if (!inst) { ok = false; break; }
                if (inst == &model.root()) { error = "cannot group the DataModel root"; ok = false; break; }
                targets.push_back(inst);
            }
            if (!ok) { respond_error(-32004, error); continue; }
            for (size_t i = 0; i < targets.size() && ok; ++i)
                for (size_t j = 0; j < targets.size(); ++j)
                    if (i != j) {
                        Instance* a = targets[i];
                        bool descendant = false;
                        for (Instance* cur = targets[j]->parent; cur; cur = cur->parent)
                            if (cur == a) descendant = true;
                        if (descendant) { error = "targets cannot contain both an instance and its descendant"; ok = false; break; }
                    }
            if (!ok) { respond_error(-32005, error); continue; }
            Instance* parent = targets.front()->parent;
            std::string parent_path;
            if (require_string(params, "parent", parent_path, error)) {
                parent = model.resolve_path(parent_path, error);
                if (!parent) { respond_error(-32004, error); continue; }
            }
            std::string name = "Model";
            if (const Json* n = params.find("name")) name = n->as_string();
            Instance& group = model.group_instances(targets, *parent, name);
            Json result = brief_instance(model, group);
            result.set("groupedCount", Json(static_cast<int64_t>(targets.size())));
            respond(std::move(result));
        } else if (method == "instance/find") {
            DataModel::FindCriteria criteria;
            if (const Json* n = params.find("name")) criteria.name = n->as_string();
            if (const Json* c = params.find("className")) criteria.class_name = c->as_string();
            if (const Json* s = params.find("substring")) criteria.substring = s->as_bool();
            Instance* within = nullptr;
            std::string within_path, error;
            if (require_string(params, "root", within_path, error)) {
                within = model.resolve_path(within_path, error);
                if (!within) { respond_error(-32004, error); continue; }
            }
            auto found = model.find(criteria, within);
            Json arr = Json::array();
            for (Instance* inst : found) arr.push(brief_instance(model, *inst));
            Json result = Json::object();
            result.set("matches", std::move(arr));
            result.set("count", Json(static_cast<int64_t>(found.size())));
            respond(std::move(result));
        } else if (method == "instance/move" || method == "instance/rotate" || method == "instance/scale") {
            std::string target, error;
            if (!require_string(params, "target", target, error)) { respond_error(-32602, error); continue; }
            Instance* inst = model.resolve_path(target, error);
            if (!inst) { respond_error(-32004, error); continue; }
            bool ok = false;
            if (method == "instance/move") {
                std::array<double, 3> delta{};
                if (!require_vec3(params, "delta", delta, error)) { respond_error(-32602, error); continue; }
                bool relative = false;
                if (const Json* r = params.find("relative")) relative = r->as_bool();
                ok = model.move(*inst, delta, relative, error);
            } else if (method == "instance/rotate") {
                std::array<double, 3> angles{};
                if (!require_vec3(params, "angles", angles, error)) { respond_error(-32602, error); continue; }
                bool world = false;
                if (const Json* w = params.find("worldSpace")) world = w->as_bool();
                ok = model.rotate(*inst, angles, world, error);
            } else {
                std::array<double, 3> factors{1, 1, 1};
                if (const Json* f = params.find("factor")) {
                    double v = f->as_double();
                    factors = {v, v, v};
                } else if (!require_vec3(params, "factors", factors, error)) { respond_error(-32602, error); continue; }
                ok = model.scale(*inst, factors, error);
            }
            if (!ok) { respond_error(-32005, error); continue; }
            Json result = brief_instance(model, *inst);
            result.set("instance", DataModel::instance_to_json(*inst));
            respond(std::move(result));
        } else if (method == "instance/pivot") {
            std::string target, error;
            if (!require_string(params, "target", target, error)) { respond_error(-32602, error); continue; }
            Instance* inst = model.resolve_path(target, error);
            if (!inst) { respond_error(-32004, error); continue; }
            if (inst->class_name != "Model") { respond_error(-32005, "pivot only applies to a Model, got " + inst->class_name); continue; }
            const Json* value_j = params.find("value");
            if (value_j) {
                Variant pivot;
                if (!Variant::from_json(*value_j, pivot, error)) { respond_error(-32602, error); continue; }
                if (!model.set_model_pivot(*inst, pivot, error)) { respond_error(-32005, error); continue; }
            }
            Json result = brief_instance(model, *inst);
            result.set("pivot", model.model_pivot(*inst).to_json());
            respond(std::move(result));
        } else if (method == "render/camera") {
            blox::SceneOptions sopts;
            if (const Json* a = params.find("assetsDir")) sopts.assets_dir = a->as_string();
            if (const Json* a = params.find("placeDir")) sopts.place_dir = a->as_string();
            blox::Scene scene = blox::build_scene(model, sopts);
            std::array<float, 3> bmin, bmax;
            scene_bounds(scene, bmin, bmax);
            Json result = Json::object();
            result.set("hasCurrentCamera", Json(scene.has_current_camera));
            if (scene.has_current_camera) {
                result.set("currentCameraName", Json(scene.current_camera_name));
                result.set("currentCamera", camera_json(scene.current_camera));
            }
            Json bounds = Json::object();
            bounds.set("min", vec3_json(bmin));
            bounds.set("max", vec3_json(bmax));
            result.set("sceneBounds", std::move(bounds));
            result.set("suggestedCamera", camera_json(frame_camera(bmin, bmax, "default")));
            result.set("partCount", Json(static_cast<int64_t>(scene.part_count)));
            result.set("drawCount", Json(static_cast<int64_t>(scene.draws.size())));
            respond(std::move(result));
        } else if (method == "render/screenshot") {
            blox::SceneOptions sopts;
            if (const Json* a = params.find("assetsDir")) sopts.assets_dir = a->as_string();
            if (const Json* a = params.find("placeDir")) sopts.place_dir = a->as_string();
            std::string quality = "high";
            if (const Json* q = params.find("quality")) quality = q->as_string();
            if (quality == "low") { sopts.sphere_segments = 12; sopts.cylinder_segments = 12; }
            blox::Scene scene = blox::build_scene(model, sopts);
            std::array<float, 3> bmin, bmax;
            scene_bounds(scene, bmin, bmax);

            blox::RenderRequest req;
            req.width = static_cast<uint32_t>(params.find("width") ? params.find("width")->as_int(800) : 800);
            req.height = static_cast<uint32_t>(params.find("height") ? params.find("height")->as_int(600) : 600);
            if (const Json* o = params.find("outputPath")) req.output_path = o->as_string();
            if (const Json* v = params.find("shadows")) req.shadows = v->as_bool(true);
            if (quality == "low") req.shadow_map_size = 512;
            else if (quality == "medium") req.shadow_map_size = 1024;
            else req.shadow_map_size = 2048;
            if (const Json* e = params.find("exposure")) scene.lighting.exposure = static_cast<float>(e->as_double());

            std::string preset = "default";
            if (const Json* pr = params.find("preset")) preset = pr->as_string();
            bool use_current = preset == "current" && scene.has_current_camera;
            blox::CameraInfo cam = use_current ? scene.current_camera : frame_camera(bmin, bmax, preset);
            if (const Json* c = params.find("camera")) {
                if (json_vec3(*c, "position", cam.position)) {}
                if (const Json* p2 = c->find("position")) { (void)p2; }
                std::array<float, 3> tmp;
                const Json* cj = c;
                if (json_vec3(*cj, "position", tmp)) cam.position = tmp;
                if (json_vec3(*cj, "lookAt", tmp)) cam.look_at = tmp;
                if (json_vec3(*cj, "up", tmp)) cam.up = tmp;
                if (const Json* f = cj->find("fovDegrees")) cam.fov_degrees = static_cast<float>(f->as_double());
                if (const Json* f = cj->find("nearPlane")) cam.near_plane = static_cast<float>(f->as_double());
                if (const Json* f = cj->find("farPlane")) cam.far_plane = static_cast<float>(f->as_double());
                if (const Json* f = cj->find("orthographic")) cam.orthographic = f->as_bool();
                if (const Json* f = cj->find("orthoHeight")) cam.ortho_height = static_cast<float>(f->as_double());
            }
            req.camera = cam;

            // Optional pixel probes are answered from the same frame.
            std::vector<std::array<int64_t, 2>> probes;
            if (const Json* pr = params.find("probePixels")) {
                if (pr->is_array()) {
                    for (const auto& item : pr->as_array()) {
                        if (item.is_array() && item.as_array().size() == 2)
                            probes.push_back({item.as_array()[0].as_int(), item.as_array()[1].as_int()});
                    }
                }
            }
            req.return_pixels = !probes.empty();
            blox::RenderResult rr = renderer.render(scene, req);
            if (!rr.ok) { respond_error(-32010, "render failed: " + rr.error); continue; }
            Json result = Json::object();
            result.set("width", Json(static_cast<int64_t>(rr.width)));
            result.set("height", Json(static_cast<int64_t>(rr.height)));
            result.set("device", Json(rr.device_name));
            result.set("drawCalls", Json(static_cast<int64_t>(rr.draw_calls)));
            result.set("triangles", Json(static_cast<int64_t>(rr.triangles)));
            result.set("renderMs", Json(rr.render_ms));
            result.set("parts", Json(static_cast<int64_t>(scene.part_count)));
            result.set("camera", camera_json(cam));
            Json skipped = Json::array();
            for (const auto& s : scene.skipped) skipped.push(Json(s));
            result.set("skipped", std::move(skipped));
            Json missing = Json::array();
            for (const auto& s : scene.missing_assets) missing.push(Json(s));
            result.set("missingAssets", std::move(missing));
            Json warnings = Json::array();
            for (const auto& w : scene.warnings) warnings.push(Json(w));
            result.set("warnings", std::move(warnings));
            if (!probes.empty()) {
                Json arr = Json::array();
                for (const auto& probe : probes) {
                    Json item = Json::object();
                    item.set("x", Json(probe[0]));
                    item.set("y", Json(probe[1]));
                    int64_t px = std::clamp<int64_t>(probe[0], 0, static_cast<int64_t>(rr.width) - 1);
                    int64_t py = std::clamp<int64_t>(probe[1], 0, static_cast<int64_t>(rr.height) - 1);
                    size_t at = (static_cast<size_t>(py) * rr.width + static_cast<size_t>(px)) * 4;
                    Json rgba = Json::array();
                    for (int k = 0; k < 4; ++k) rgba.push(Json(static_cast<int64_t>(rr.rgba[at + k])));
                    item.set("rgba", std::move(rgba));
                    arr.push(std::move(item));
                }
                result.set("probes", std::move(arr));
            }
            respond(std::move(result));
        } else if (method == "ui/textBounds") {
            std::string text;
            if (const Json* t = params.find("text")) text = t->as_string();
            double size = 14.0;
            if (const Json* v = params.find("size")) size = v->as_double(size);
            double width = 1e9;
            if (const Json* v = params.find("width")) width = v->as_double(width);
            float measured = blox::measure_text(text, static_cast<float>(size), "");
            int lines = 1;
            if (width > 0 && measured > width) {
                // Greedy word wrap, mirroring the layout engine.
                float space_w = blox::measure_text(" ", static_cast<float>(size), "");
                float current = 0;
                size_t at = 0;
                while (at <= text.size()) {
                    size_t sp = text.find(' ', at);
                    std::string word = text.substr(at, sp == std::string::npos ? sp : sp - at);
                    float ww = blox::measure_text(word, static_cast<float>(size), "");
                    if (current > 0 && current + space_w + ww > width) { lines++; current = ww; }
                    else current += (current > 0 ? space_w : 0) + ww;
                    if (sp == std::string::npos) break;
                    at = sp + 1;
                }
            }
            float used_width = width > 0 ? std::min(measured, static_cast<float>(width)) : measured;
            if (lines > 1) {
                // Widest wrapped line recomputed conservatively.
                used_width = static_cast<float>(width);
            }
            Json result = Json::object();
            result.set("x", Json(static_cast<double>(used_width)));
            result.set("y", Json(lines * size * 1.4));
            result.set("lines", Json(static_cast<int64_t>(lines)));
            result.set("font", Json("DejaVu Sans (substitute for Roblox fonts; metrics differ)"));
            respond(std::move(result));
        } else if (method == "ui/layout" || method == "ui/validate") {
            blox::UiOptions uopts;
            uopts.width = static_cast<uint32_t>(params.find("width") ? params.find("width")->as_int(1280) : 1280);
            uopts.height = static_cast<uint32_t>(params.find("height") ? params.find("height")->as_int(720) : 720);
            if (const Json* g = params.find("gui")) uopts.screen_gui = g->as_string();
            if (const Json* v = params.find("ignoreInset")) uopts.ignore_gui_inset = v->as_bool();
            if (const Json* a = params.find("assetsDir")) uopts.assets_dir = a->as_string();
            if (const Json* a = params.find("placeDir")) uopts.place_dir = a->as_string();
            if (const Json* f = params.find("fontPath")) uopts.font_path = f->as_string();
            blox::UiScene ui_scene = blox::build_ui_scene(model, uopts);
            Json rects = Json::array();
            for (const auto& rect : ui_scene.rects) {
                Json item = Json::object();
                item.set("path", Json(rect.path));
                item.set("className", Json(rect.class_name));
                item.set("name", Json(rect.name));
                item.set("x", Json(static_cast<double>(rect.x)));
                item.set("y", Json(static_cast<double>(rect.y)));
                item.set("width", Json(static_cast<double>(rect.width)));
                item.set("height", Json(static_cast<double>(rect.height)));
                item.set("zindex", Json(static_cast<double>(rect.zindex)));
                item.set("visible", Json(rect.visible));
                item.set("order", Json(static_cast<int64_t>(rect.order)));
                item.set("parent", Json(rect.parent_path));
                if (!rect.text.empty()) item.set("text", Json(rect.text));
                rects.push(std::move(item));
            }
            Json result = Json::object();
            result.set("rects", std::move(rects));
            result.set("quadCount", Json(static_cast<int64_t>(ui_scene.quads.size())));
            Json warns = Json::array();
            for (const auto& w : ui_scene.warnings) warns.push(Json(w));
            result.set("warnings", std::move(warns));
            if (method == "ui/validate") {
                auto issues = blox::validate_ui(ui_scene, model);
                Json arr = Json::array();
                int64_t errors = 0;
                for (const auto& issue : issues) {
                    if (issue.severity == "error") ++errors;
                    Json item = Json::object();
                    item.set("severity", Json(issue.severity));
                    item.set("path", Json(issue.path));
                    item.set("message", Json(issue.message));
                    arr.push(std::move(item));
                }
                result.set("valid", Json(errors == 0));
                result.set("errorCount", Json(errors));
                result.set("issues", std::move(arr));
            }
            respond(std::move(result));
        } else if (method == "ui/render") {
            blox::UiOptions uopts;
            uopts.width = static_cast<uint32_t>(params.find("width") ? params.find("width")->as_int(1280) : 1280);
            uopts.height = static_cast<uint32_t>(params.find("height") ? params.find("height")->as_int(720) : 720);
            if (const Json* g = params.find("gui")) uopts.screen_gui = g->as_string();
            if (const Json* v = params.find("ignoreInset")) uopts.ignore_gui_inset = v->as_bool();
            if (const Json* a = params.find("assetsDir")) uopts.assets_dir = a->as_string();
            if (const Json* a = params.find("placeDir")) uopts.place_dir = a->as_string();
            if (const Json* f = params.find("fontPath")) uopts.font_path = f->as_string();

            blox::UiRenderRequest ureq;
            ureq.width = uopts.width;
            ureq.height = uopts.height;
            if (const Json* o = params.find("outputPath")) ureq.output_path = o->as_string();

            bool with_scene = false;
            if (const Json* w = params.find("withScene")) with_scene = w->as_bool();
            // Camera framing shared with the 3D pass (also projects billboards).
            blox::SceneOptions sopts;
            sopts.assets_dir = uopts.assets_dir;
            sopts.place_dir = uopts.place_dir;
            blox::Scene scene3d = blox::build_scene(model, sopts);
            std::array<float, 3> bmin, bmax;
            scene_bounds(scene3d, bmin, bmax);
            blox::CameraInfo cam = frame_camera(bmin, bmax, "default");
            if (const Json* c = params.find("camera")) {
                std::array<float, 3> tmp;
                if (json_vec3(*c, "position", tmp)) cam.position = tmp;
                if (json_vec3(*c, "lookAt", tmp)) cam.look_at = tmp;
                if (const Json* f = c->find("fovDegrees")) cam.fov_degrees = static_cast<float>(f->as_double());
            }
            uopts.camera_position = cam.position;
            uopts.camera_look_at = cam.look_at;
            uopts.camera_fov_degrees = cam.fov_degrees;

            blox::UiScene ui_scene = blox::build_ui_scene(model, uopts);
            if (with_scene) {
                blox::RenderRequest sreq;
                sreq.width = ureq.width;
                sreq.height = ureq.height;
                sreq.camera = cam;
                sreq.return_pixels = true;
                blox::RenderResult srr = renderer.render(scene3d, sreq);
                if (!srr.ok) { respond_error(-32010, "scene render for composite failed: " + srr.error); continue; }
                ureq.background_rgba = std::move(srr.rgba);
            }
            std::vector<std::array<int64_t, 2>> probes;
            if (const Json* pr = params.find("probePixels")) {
                if (pr->is_array()) {
                    for (const auto& item : pr->as_array()) {
                        if (item.is_array() && item.as_array().size() == 2)
                            probes.push_back({item.as_array()[0].as_int(), item.as_array()[1].as_int()});
                    }
                }
            }
            ureq.return_pixels = !probes.empty();
            blox::RenderResult rr = renderer.render_ui(ui_scene, ureq);
            if (!rr.ok) { respond_error(-32010, "ui render failed: " + rr.error); continue; }
            Json result = Json::object();
            result.set("width", Json(static_cast<int64_t>(rr.width)));
            result.set("height", Json(static_cast<int64_t>(rr.height)));
            result.set("device", Json(rr.device_name));
            result.set("drawCalls", Json(static_cast<int64_t>(rr.draw_calls)));
            result.set("triangles", Json(static_cast<int64_t>(rr.triangles)));
            result.set("renderMs", Json(rr.render_ms));
            result.set("quadCount", Json(static_cast<int64_t>(ui_scene.quads.size())));
            result.set("rectCount", Json(static_cast<int64_t>(ui_scene.rects.size())));
            Json warns = Json::array();
            for (const auto& w : ui_scene.warnings) warns.push(Json(w));
            result.set("warnings", std::move(warns));
            if (!probes.empty()) {
                Json arr = Json::array();
                for (const auto& probe : probes) {
                    Json item = Json::object();
                    item.set("x", Json(probe[0]));
                    item.set("y", Json(probe[1]));
                    int64_t px = std::clamp<int64_t>(probe[0], 0, static_cast<int64_t>(rr.width) - 1);
                    int64_t py = std::clamp<int64_t>(probe[1], 0, static_cast<int64_t>(rr.height) - 1);
                    size_t at = (static_cast<size_t>(py) * rr.width + static_cast<size_t>(px)) * 4;
                    Json rgba = Json::array();
                    for (int k = 0; k < 4; ++k) rgba.push(Json(static_cast<int64_t>(rr.rgba[at + k])));
                    item.set("rgba", std::move(rgba));
                    arr.push(std::move(item));
                }
                result.set("probes", std::move(arr));
            }
            respond(std::move(result));
        } else if (method == "validate") {
            auto issues = model.validate();
            int64_t errors = 0;
            Json arr = Json::array();
            for (const auto& issue : issues) {
                if (issue.severity == "error") ++errors;
                Json item = Json::object();
                item.set("severity", Json(issue.severity));
                item.set("path", Json(issue.path));
                item.set("message", Json(issue.message));
                arr.push(std::move(item));
            }
            Json result = Json::object();
            result.set("valid", Json(errors == 0));
            result.set("errorCount", Json(errors));
            result.set("issueCount", Json(static_cast<int64_t>(issues.size())));
            result.set("issues", std::move(arr));
            result.set("instanceCount", Json(static_cast<int64_t>(model.instance_count())));
            respond(std::move(result));
        } else {
            respond_error(-32601, "method not found: " + method);
        }
    }
    return 0;
}
