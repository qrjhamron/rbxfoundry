#include "blox/ipc/server.hpp"
#include "server_internal.hpp"
#include "blox/rendering/renderer.hpp"
#include "blox/scripting/script_tools.hpp"

namespace blox::ipc {

EngineServer::Reply EngineServer::handle_script(const std::string& method, const Json& params) {
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
            return Reply::ok(std::move(result));
        }
        if (method == "parity/imagediff") {
            auto str_param = [&](const char* key) -> std::string {
                const Json* v = params.find(key);
                return v && v->is_string() ? v->as_string() : std::string();
            };
            blox::ImageDiff diff;
            std::string diff_error;
            if (!blox::image_diff(str_param("a"), str_param("b"), str_param("heatmap"), diff, diff_error)) {
                return Reply::error(-32002, diff_error);
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
            return Reply::ok(std::move(result));
        }


    return Reply::error(-32601, "method not found: " + method);
}

} // namespace blox::ipc
