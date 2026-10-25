#pragma once
#include <string>

#include "blox/datamodel.hpp"
#include "blox/renderer.hpp"

namespace blox {

// Options controlling how a DataModel becomes a renderable Scene.
struct SceneOptions {
    // Directory searched for local mesh/texture assets ("<id>.mesh",
    // "<id>.png", or the raw file name from a path-like id).
    std::string assets_dir;
    // Place file directory, used to resolve path-like asset ids.
    std::string place_dir;
    uint32_t sphere_segments = 24;
    uint32_t cylinder_segments = 24;
};

// Extracts drawables, lighting and the current camera from the DataModel.
// Anything that cannot be represented is reported in Scene::skipped /
// Scene::missing_assets, never silently replaced with fake geometry.
Scene build_scene(const DataModel& model, const SceneOptions& options);

// Loads a MeshPart/SpecialMesh asset into `out` (local space). Implemented
// in mesh.cpp. Returns false with `error` when the asset cannot be resolved
// or parsed; the geometry is never faked.
// Resolves a texture asset id to a local PNG path (see mesh.cpp).
std::string resolve_texture_asset(const std::string& raw_id, const SceneOptions& options, bool& found);

bool load_mesh_geometry(const std::string& mesh_id, const std::string& texture_id,
                        const SceneOptions& options, const Instance& part,
                        DrawItem& out, std::string& error);

// Local-space geometry generators (positions/normals/uvs/indices appended).
// `size` is the part's full Size. Roblox fitting rules:
//  - Block: the box itself.
//  - Ball (sphere): diameter = min(Size components), centred.
//  - Cylinder: axis along X, diameter = min(Size.y, Size.z), length = Size.x.
void gen_box(const std::array<double, 3>& size, DrawItem& out);
void gen_sphere(const std::array<double, 3>& size, uint32_t segments, DrawItem& out);
void gen_cylinder(const std::array<double, 3>& size, uint32_t segments, DrawItem& out);
void gen_wedge(const std::array<double, 3>& size, DrawItem& out);
void gen_corner_wedge(const std::array<double, 3>& size, DrawItem& out);

} // namespace blox
