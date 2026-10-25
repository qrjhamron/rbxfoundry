#include "blox/ipc/server.hpp"
#include "server_internal.hpp"
#include "blox/terrain/terrain.hpp"
#include <map>

namespace blox::ipc {

EngineServer::Reply EngineServer::handle_terrain(const std::string& method, const Json& params) {
            std::string error;
            Instance* workspace = nullptr;
            for (const auto& child : model_.root().children)
                if (child->class_name == "Workspace") workspace = child.get();
            if (!workspace) { return Reply::error(-32010, "place has no Workspace"); }
            Instance* terrain = find_terrain(model_);
            auto ensure_terrain = [&]() -> Instance* {
                if (terrain) return terrain;
                Instance& created = model_.add_instance(*workspace, "Terrain", "Terrain");
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
                    if (!terrain_load(*terrain, grid, error)) { return Reply::error(-32025, error); }
                    voxels = static_cast<int64_t>(grid.cells.size());
                    std::map<uint16_t, int64_t> counts;
                    for (const auto& [key, value] : grid.cells) counts[value.first]++;
                    for (const auto& [mat, n] : counts)
                        materials.set(std::to_string(mat), Json(n));
                }
                result.set("voxels", Json(voxels));
                result.set("materialsByEnumValue", std::move(materials));
                result.set("voxelSize", Json(blox::kVoxelSize));
                return Reply::ok(std::move(result));
            }
            if (method == "terrain/fill" || method == "terrain/fillBall") {
                auto vec3_param = [&](const char* key, std::array<double, 3>& out) {
                    const Json* v = params.find(key);
                    if (!v || !v->is_array() || v->as_array().size() != 3) return false;
                    out = {v->as_array()[0].as_double(), v->as_array()[1].as_double(), v->as_array()[2].as_double()};
                    return true;
                };
                std::array<double, 3> center{};
                if (!vec3_param("center", center)) { return Reply::error(-32602, "terrain fill needs center [x,y,z]"); }
                int64_t material = 1280;
                if (const Json* m = params.find("material")) material = m->as_int(material);
                Instance* target = ensure_terrain();
                blox::TerrainGrid grid;
                if (!terrain_load(*target, grid, error)) { return Reply::error(-32025, error); }
                if (method == "terrain/fill") {
                    std::array<double, 3> size{4, 4, 4};
                    if (!vec3_param("size", size)) { return Reply::error(-32602, "terrain/fill needs size [x,y,z]"); }
                    terrain_fill_block(grid, center, size, static_cast<uint16_t>(material));
                } else {
                    double radius = 4.0;
                    if (const Json* r = params.find("radius")) radius = r->as_double(radius);
                    terrain_fill_ball(grid, center, radius, static_cast<uint16_t>(material));
                }
                if (!terrain_store(model_, *target, grid, error)) { return Reply::error(-32025, error); }
                Json result = Json::object();
                result.set("voxels", Json(static_cast<int64_t>(grid.cells.size())));
                return Reply::ok(std::move(result));
            }
            if (method == "terrain/flatten" || method == "terrain/smooth" || method == "terrain/paint") {
                auto vec3_param = [&](const char* key, std::array<double, 3>& out) {
                    const Json* v = params.find(key);
                    if (!v || !v->is_array() || v->as_array().size() != 3) return false;
                    out = {v->as_array()[0].as_double(), v->as_array()[1].as_double(), v->as_array()[2].as_double()};
                    return true;
                };
                std::array<double, 3> center{}, size{16, 16, 16};
                if (!vec3_param("center", center)) { return Reply::error(-32602, "terrain op needs center [x,y,z]"); }
                if (!vec3_param("size", size)) { return Reply::error(-32602, "terrain op needs size [x,y,z]"); }
                int64_t material = 1280;
                if (const Json* m = params.find("material")) material = m->as_int(material);
                Instance* target = ensure_terrain();
                blox::TerrainGrid grid;
                if (!terrain_load(*target, grid, error)) { return Reply::error(-32025, error); }
                if (method == "terrain/flatten") {
                    double height = center[1];
                    if (const Json* h = params.find("height")) height = h->as_double(height);
                    terrain_flatten(grid, center, size, height, static_cast<uint16_t>(material));
                } else if (method == "terrain/smooth") {
                    terrain_smooth(grid, center, size);
                } else {
                    terrain_paint(grid, center, size, static_cast<uint16_t>(material));
                }
                if (!terrain_store(model_, *target, grid, error)) { return Reply::error(-32025, error); }
                Json result = Json::object();
                result.set("voxels", Json(static_cast<int64_t>(grid.cells.size())));
                return Reply::ok(std::move(result));
            }
            if (method == "terrain/clear") {
                if (terrain) {
                    blox::TerrainGrid grid;
                    if (!terrain_store(model_, *terrain, grid, error)) { return Reply::error(-32025, error); }
                }
                Json result = Json::object();
                result.set("voxels", Json(static_cast<int64_t>(0)));
                return Reply::ok(std::move(result));
            }
            return Reply::error(-32601, "unknown terrain method '" + method + "'");

}

} // namespace blox::ipc
