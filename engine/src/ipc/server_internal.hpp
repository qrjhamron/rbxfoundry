#pragma once
// Shared JSON helpers for the IPC handlers. Not a public API.
#include <array>
#include <string>

#include "blox/core/json.hpp"
#include "blox/datamodel/datamodel.hpp"
#include "blox/rendering/renderer.hpp"

namespace blox::ipc {
bool require_string(const Json& params, const char* key, std::string& out, std::string& error);
bool require_vec3(const Json& params, const char* key, std::array<double, 3>& out, std::string& error);
bool json_vec3(const Json& params, const char* key, std::array<float, 3>& out);
Json vec3_json(const std::array<float, 3>& v);
Json camera_json(const CameraInfo& cam);
void scene_bounds(const Scene& scene, std::array<float, 3>& bmin, std::array<float, 3>& bmax);
CameraInfo frame_camera(const std::array<float, 3>& bmin, const std::array<float, 3>& bmax,
                        const std::string& preset);
Json brief_instance(DataModel& model, Instance& inst);
} // namespace blox::ipc
