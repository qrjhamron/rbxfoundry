#include "blox/ipc/server.hpp"
#include "server_internal.hpp"

namespace blox::ipc {

EngineServer::Reply EngineServer::handle_debug(const std::string& method, const Json& params) {
            auto debug_json = [&](int frame, bool with_vars) {
                std::vector<blox::Runtime::DebugVariable> vars;
                auto status = runtime_->debug_status(frame, with_vars ? &vars : nullptr);
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
                runtime_->debug_set_breakpoints(bps);
                runtime_->start_scripts();
                return Reply::ok(debug_json(0, true));
            }
            if (method == "debug/state") {
                int frame = 0;
                if (const Json* f = params.find("frame")) frame = static_cast<int>(f->as_int());
                return Reply::ok(debug_json(frame, true));
            }
            if (method == "debug/resume") {
                std::string mode = "continue";
                if (const Json* m = params.find("mode")) mode = m->as_string();
                if (mode != "continue" && mode != "stepInto" && mode != "stepOver" && mode != "stepOut") {
                    return Reply::error(-32602, "debug/resume mode must be continue|stepInto|stepOver|stepOut");
                }
                if (!runtime_->debug_resume(mode)) {
                    return Reply::error(-32026, "no paused thread to resume");
                }
                return Reply::ok(debug_json(0, true));
            }
            return Reply::error(-32601, "unknown debug method '" + method + "'");

}

} // namespace blox::ipc
