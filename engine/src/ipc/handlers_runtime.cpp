#include "blox/ipc/server.hpp"
#include "server_internal.hpp"
#include <algorithm>

namespace blox::ipc {

EngineServer::Reply EngineServer::handle_runtime(const std::string& method, const Json& params) {
            if (method == "runtime/begin") {
                // Copy the edit model into a fresh runtime model_.
                Json tree = model_.snapshot();
                std::string error;
                if (!runtime_model_.load_snapshot(tree, error)) { return Reply::error(-32020, error); }
                runtime_ = std::make_unique<blox::Runtime>(runtime_model_);
        if (const Json* ds = params.find("dataStoreDir")) {
            if (ds->is_string()) runtime_->set_datastore_dir(ds->as_string());
        }
                runtime_active_ = true;
                outgoing_remotes_.clear();
                runtime_->set_remote_sink([&](const blox::Runtime::RemoteCall& call) {
                    Json item = Json::object();
                    item.set("path", Json(runtime_model_.path_of(*call.remote)));
                    item.set("kind", Json(static_cast<int64_t>(call.kind)));
                    item.set("fromPlayer", Json(static_cast<int64_t>(call.from_player)));
                    item.set("invokeId", Json(static_cast<int64_t>(call.invoke_id)));
                    Json args;
                    std::string parse_error;
                    if (Json::parse(call.args_json, args, parse_error)) item.set("args", std::move(args));
                    outgoing_remotes_.push_back(std::move(item));
                });
                if (const Json* ctx = params.find("netContext")) {
                    std::string c = ctx->as_string();
                    int pidx = -1;
                    if (const Json* pi = params.find("playerIndex")) pidx = static_cast<int>(pi->as_int(-1));
                    if (c == "server") runtime_->set_net_context(blox::Runtime::NetContext::Server, pidx);
                    else if (c == "client") {
                        runtime_->set_net_context(blox::Runtime::NetContext::Client, pidx);
                        if (const Json* pn = params.find("playerName")) {
                            std::string perr; Instance* players = runtime_model_.resolve_path("Players", perr);
                            if (players) {
                                for (const auto& child : players->children)
                                    if (child->name == pn->as_string()) { runtime_->set_local_player(child.get()); break; }
                            }
                        }
                    }
                }
                Json result = Json::object();
                result.set("active", Json(true));
                result.set("instances", Json(static_cast<int64_t>(runtime_model_.instance_count())));
                return Reply::ok(std::move(result));
            }
            if (method == "runtime/reset") {
                runtime_.reset();
                runtime_active_ = false;
                Json result = Json::object();
                result.set("active", Json(false));
                return Reply::ok(std::move(result));
            }
            if (!runtime_active_ || !runtime_) { return Reply::error(-32021, "no runtime session: call 'runtime/begin' first"); }
            if (method == "runtime/runScripts") {
                runtime_->start_scripts();
                return Reply::ok(runtime_state());
            }
            if (method == "runtime/eval") {
                std::string source;
                std::string error;
                if (!require_string(params, "source", source, error)) { return Reply::error(-32602, error); }
                std::string name = "eval";
                if (const Json* n = params.find("name")) name = n->as_string();
                std::string error_msg, result_json;
                bool ok = runtime_->eval(source, name, error_msg, result_json);
                Json result = runtime_state();
                result.set("ok", Json(ok));
                if (!ok) result.set("error", Json(error_msg));
                Json values;
                std::string parse_error;
                if (Json::parse(result_json.empty() ? "[]" : result_json, values, parse_error))
                    result.set("returns", std::move(values));
                return Reply::ok(std::move(result));
            }
            if (method == "runtime/step") {
                double dt = 1.0 / 60.0;
                if (const Json* d = params.find("dt")) dt = d->as_double(dt);
                int steps = 1;
                if (const Json* n = params.find("steps")) steps = static_cast<int>(n->as_int(1));
                for (int i = 0; i < steps; ++i) runtime_->step(dt);
                return Reply::ok(runtime_state());
            }
            if (method == "runtime/state") {
                return Reply::ok(runtime_state());
            }
            if (method == "runtime/click" || method == "runtime/hover") {
                double x = 0, y = 0;
                uint32_t vw = 1280, vh = 720;
                if (const Json* v = params.find("x")) x = v->as_double();
                if (const Json* v = params.find("y")) y = v->as_double();
                if (const Json* v = params.find("width")) vw = static_cast<uint32_t>(v->as_int(vw));
                if (const Json* v = params.find("height")) vh = static_cast<uint32_t>(v->as_int(vh));
                auto hit = method == "runtime/click" ? runtime_->gui_click(x, y, vw, vh)
                                                     : runtime_->gui_hover(x, y, vw, vh);
                Json result = runtime_state();
                Json hit_json = Json::object();
                hit_json.set("hit", Json(hit.hit));
                hit_json.set("path", Json(hit.path));
                hit_json.set("className", Json(hit.class_name));
                hit_json.set("event", Json(hit.event));
                result.set("hit", std::move(hit_json));
                return Reply::ok(std::move(result));
            }
            if (method == "runtime/typeText") {
                std::string text;
                bool submit = false;
                if (const Json* v = params.find("text")) text = v->as_string();
                if (const Json* v = params.find("submit")) submit = v->as_bool();
                Json result = runtime_state();
                result.set("typed", Json(runtime_->gui_type_text(text, submit)));
                return Reply::ok(std::move(result));
            }
            if (method == "runtime/keys") {
                // UserInputService key state (play mode input feed).
                if (const Json* keys = params.find("keys")) {
                    if (keys->is_object()) {
                        for (const auto& [name, down] : keys->as_object())
                            runtime_->set_key_state(name, down.as_bool());
                    }
                }
                return Reply::ok(runtime_state());
            }
            if (method == "runtime/snapshot") {
                Json result = Json::object();
                result.set("tree", runtime_model_.snapshot());
                return Reply::ok(std::move(result));
            }
            if (method == "runtime/netDeliver") {
                std::string path;
                if (const Json* v = params.find("path")) path = v->as_string();
                std::string rerr; Instance* remote = runtime_model_.resolve_path(path, rerr);
                if (!remote) { return Reply::error(-32023, "no remote instance at path '" + path + "'"); }
                Instance* from_player = nullptr;
                if (const Json* fp = params.find("fromPlayerPath"))
                    { std::string ferr; from_player = runtime_model_.resolve_path(fp->as_string(), ferr); }
                std::string args = "[]";
                if (const Json* a = params.find("args")) args = a->dump();
                runtime_->deliver_remote(remote, from_player, args);
                return Reply::ok(runtime_state());
            }
            if (method == "runtime/netInvokeHandler") {
                std::string path;
                if (const Json* v = params.find("path")) path = v->as_string();
                std::string rerr; Instance* remote = runtime_model_.resolve_path(path, rerr);
                if (!remote) { return Reply::error(-32023, "no remote instance at path '" + path + "'"); }
                Instance* from_player = nullptr;
                if (const Json* fp = params.find("fromPlayerPath"))
                    { std::string ferr; from_player = runtime_model_.resolve_path(fp->as_string(), ferr); }
                std::string args = "[]";
                if (const Json* a = params.find("args")) args = a->dump();
                bool ok = false;
                std::string result = runtime_->invoke_remote_handler(remote, from_player, args, ok);
                Json out = Json::object();
                out.set("ok", Json(ok));
                Json values;
                std::string parse_error;
                if (ok && Json::parse(result.empty() ? "[]" : result, values, parse_error))
                    out.set("result", std::move(values));
                return Reply::ok(std::move(out));
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
                runtime_->complete_invoke(invoke_id, result, ok, error);
                return Reply::ok(runtime_state());
            }
            if (method == "runtime/setProperty") {
                std::string path;
                if (const Json* v = params.find("path")) path = v->as_string();
                std::string ierr;
                Instance* inst = runtime_model_.resolve_path(path, ierr);
                if (!inst) { return Reply::error(-32023, "no instance at path '" + path + "'"); }
                const Json* prop = params.find("property");
                const Json* value = params.find("value");
                if (!prop || !value) { return Reply::error(-32602, "setProperty needs 'property' and 'value'"); }
                Variant variant;
                std::string error;
                if (!Variant::from_json(*value, variant, error) ||
                    !runtime_model_.set_property(*inst, prop->as_string(), variant, error)) {
                    return Reply::error(-32024, error);
                }
                return Reply::ok(runtime_state());
            }
            if (method == "runtime/addInstance") {
                std::string parent_path;
                if (const Json* v = params.find("parentPath")) parent_path = v->as_string();
                std::string ierr;
                Instance* parent = runtime_model_.resolve_path(parent_path, ierr);
                if (!parent) { return Reply::error(-32023, "no instance at path '" + parent_path + "'"); }
                std::string klass = "Part";
                if (const Json* v = params.find("class")) klass = v->as_string();
                std::string iname;
                if (const Json* v = params.find("name")) iname = v->as_string();
                Instance& created = runtime_model_.add_instance(*parent, klass, iname);
                if (const Json* props = params.find("properties")) {
                    if (props->is_object()) {
                        for (const auto& [key, value] : props->as_object()) {
                            Variant variant;
                            std::string error;
                            if (Variant::from_json(value, variant, error))
                                runtime_model_.set_property(created, key, variant, error);
                        }
                    }
                }
                Instance* raw = &created;
                Json result = Json::object();
                result.set("path", Json(runtime_model_.path_of(*raw)));
                return Reply::ok(std::move(result));
            }
            if (method == "runtime/removeInstance") {
                std::string path;
                if (const Json* v = params.find("path")) path = v->as_string();
                std::string ierr;
                Instance* inst = runtime_model_.resolve_path(path, ierr);
                if (!inst || !inst->parent) { return Reply::error(-32023, "no removable instance at path '" + path + "'"); }
                Instance* parent = inst->parent;
                parent->children.erase(std::remove_if(parent->children.begin(), parent->children.end(),
                    [&](const std::unique_ptr<Instance>& c) { return c.get() == inst; }), parent->children.end());
                return Reply::ok(runtime_state());
            }
            if (method == "runtime/fireSignal") {
                std::string path;
                if (const Json* v = params.find("path")) path = v->as_string();
                std::string ierr; Instance* inst = runtime_model_.resolve_path(path, ierr);
                if (!inst) { return Reply::error(-32023, "no instance at path '" + path + "'"); }
                std::string signal = "Changed";
                if (const Json* v = params.find("signal")) signal = v->as_string();
                std::string args = "[]";
                if (const Json* a = params.find("args")) args = a->dump();
                runtime_->fire_signal_json(inst, signal, args);
                return Reply::ok(runtime_state());
            }
            return Reply::error(-32601, "unknown runtime method '" + method + "'");


}

} // namespace blox::ipc
