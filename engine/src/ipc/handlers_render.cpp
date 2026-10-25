#include "blox/ipc/server.hpp"
#include "server_internal.hpp"
#include "blox/gui/ui.hpp"
#include "blox/rendering/scene.hpp"
#include <algorithm>
#include <cmath>

namespace blox::ipc {

EngineServer::Reply EngineServer::handle_render(const std::string& method, const Json& params) {
        if (method == "render/camera") {
            blox::SceneOptions sopts;
            if (const Json* a = params.find("assetsDir")) sopts.assets_dir = a->as_string();
            if (const Json* a = params.find("placeDir")) sopts.place_dir = a->as_string();
            blox::Scene scene = blox::build_scene(model_, sopts);
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
            return Reply::ok(std::move(result));
        } else if (method == "render/screenshot") {
            blox::SceneOptions sopts;
            if (const Json* a = params.find("assetsDir")) sopts.assets_dir = a->as_string();
            if (const Json* a = params.find("placeDir")) sopts.place_dir = a->as_string();
            std::string quality = "high";
            if (const Json* q = params.find("quality")) quality = q->as_string();
            if (quality == "low") { sopts.sphere_segments = 12; sopts.cylinder_segments = 12; }
            blox::Scene scene = blox::build_scene(model_, sopts);
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
            blox::RenderResult rr = renderer_.render(scene, req);
            if (!rr.ok) { return Reply::error(-32010, "render failed: " + rr.error); }
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
            return Reply::ok(std::move(result));
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
            return Reply::ok(std::move(result));
        } else if (method == "ui/layout" || method == "ui/validate") {
            blox::UiOptions uopts;
            uopts.width = static_cast<uint32_t>(params.find("width") ? params.find("width")->as_int(1280) : 1280);
            uopts.height = static_cast<uint32_t>(params.find("height") ? params.find("height")->as_int(720) : 720);
            if (const Json* g = params.find("gui")) uopts.screen_gui = g->as_string();
            if (const Json* v = params.find("ignoreInset")) uopts.ignore_gui_inset = v->as_bool();
            if (const Json* a = params.find("assetsDir")) uopts.assets_dir = a->as_string();
            if (const Json* a = params.find("placeDir")) uopts.place_dir = a->as_string();
            if (const Json* f = params.find("fontPath")) uopts.font_path = f->as_string();
            blox::UiScene ui_scene = blox::build_ui_scene(model_, uopts);
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
                auto issues = blox::validate_ui(ui_scene, model_);
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
            return Reply::ok(std::move(result));
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
            blox::Scene scene3d = blox::build_scene(model_, sopts);
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

            blox::UiScene ui_scene = blox::build_ui_scene(model_, uopts);
            if (with_scene) {
                blox::RenderRequest sreq;
                sreq.width = ureq.width;
                sreq.height = ureq.height;
                sreq.camera = cam;
                sreq.return_pixels = true;
                blox::RenderResult srr = renderer_.render(scene3d, sreq);
                if (!srr.ok) { return Reply::error(-32010, "scene render for composite failed: " + srr.error); }
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
            blox::RenderResult rr = renderer_.render_ui(ui_scene, ureq);
            if (!rr.ok) { return Reply::error(-32010, "ui render failed: " + rr.error); }
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
            return Reply::ok(std::move(result));
        } else if (method == "validate") {
            auto issues = model_.validate();
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
            result.set("instanceCount", Json(static_cast<int64_t>(model_.instance_count())));
            return Reply::ok(std::move(result));
        } else {
            return Reply::error(-32601, "method not found: " + method);
        }

}

} // namespace blox::ipc
