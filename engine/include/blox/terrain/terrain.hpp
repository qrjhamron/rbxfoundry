#pragma once
#include <array>
#include <cstdint>
#include <map>
#include <string>
#include <tuple>

#include "blox/datamodel/datamodel.hpp"

namespace blox {

// Roblox terrain voxel data (Phase 11). Voxels are 4x4x4 studs. The
// grid lives on the Workspace Terrain instance in a BinaryString
// property named "BloxVoxels" (base64 of the documented "BTR1"
// encoding), so it round-trips losslessly through .rbxl/.rbxlx with
// the rest of the place. Genuine Roblox Studio terrain chunks use a
// proprietary layout BloxCLI cannot read; that gap is documented in
// COMPATIBILITY.md rather than papered over.

inline constexpr double kVoxelSize = 4.0;

struct TerrainGrid {
    // (x, y, z) voxel coords -> (Enum.Material value, occupancy 0..255)
    std::map<std::tuple<int, int, int>, std::pair<uint16_t, uint8_t>> cells;

    bool empty() const { return cells.empty(); }
    bool solid(int x, int y, int z) const {
        auto it = cells.find({x, y, z});
        return it != cells.end() && it->second.second > 0;
    }
};

// World-space centre of a voxel.
std::array<double, 3> voxel_center(int x, int y, int z);

// Finds the Terrain instance directly under Workspace (may be null).
Instance* find_terrain(DataModel& model);
const Instance* find_terrain(const DataModel& model);

// Decodes the grid from the Terrain instance (empty grid when unset).
bool terrain_load(const Instance& terrain, TerrainGrid& out, std::string& error);

// Encodes and stores the grid on the Terrain instance.
bool terrain_store(DataModel& model, Instance& terrain, const TerrainGrid& grid, std::string& error);

// Fills voxels intersecting the axis-aligned world box (CFrame rotation
// is ignored, documented) — or removes them when material == 0 (Air).
void terrain_fill_block(TerrainGrid& grid, std::array<double, 3> center,
                        std::array<double, 3> size, uint16_t material);
void terrain_fill_ball(TerrainGrid& grid, std::array<double, 3> center, double radius,
                       uint16_t material);
// Cylinder around the Y axis through `center` (CFrame rotation ignored,
// like FillBlock): voxels whose centre lies within `radius` in XZ and
// within `height/2` in Y.
void terrain_fill_cylinder(TerrainGrid& grid, std::array<double, 3> center,
                           double radius, double height, uint16_t material);
// Wedge filling the lower half of the box in Z: a voxel is filled when
// its centre is below the diagonal plane from the box's back-bottom to
// its front-top (axis-aligned approximation, rotation ignored).
void terrain_fill_wedge(TerrainGrid& grid, std::array<double, 3> center,
                        std::array<double, 3> size, uint16_t material);

// Studio Terrain Editor operations (Phase 21), all in voxel space:
// flatten: level the region's columns to `height` (world Y) — material
// fills below, air above. smooth: one cellular pass (a voxel becomes
// solid when most of its 26 neighbours are solid, taking the majority
// material; otherwise it is removed). paint: replace the material of
// solid voxels in the region (material 0 erases, like fill).
void terrain_flatten(TerrainGrid& grid, std::array<double, 3> center,
                     std::array<double, 3> size, double height, uint16_t material);
void terrain_smooth(TerrainGrid& grid, std::array<double, 3> center,
                    std::array<double, 3> size);
void terrain_paint(TerrainGrid& grid, std::array<double, 3> center,
                   std::array<double, 3> size, uint16_t material);

// sRGB colour used to render a terrain material (documented palette:
// these are BloxCLI's approximations, not Studio's textures).
std::array<float, 3> terrain_material_color(uint16_t material);

} // namespace blox
