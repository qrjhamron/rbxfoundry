#include "blox/terrain/terrain.hpp"

#include <cmath>
#include <cstring>

namespace blox {
namespace {

std::string base64_encode(const std::string& data) {
    static const char* kTable = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    std::string out;
    for (size_t i = 0; i < data.size(); i += 3) {
        uint32_t n = static_cast<uint8_t>(data[i]) << 16;
        if (i + 1 < data.size()) n |= static_cast<uint8_t>(data[i + 1]) << 8;
        if (i + 2 < data.size()) n |= static_cast<uint8_t>(data[i + 2]);
        out += kTable[(n >> 18) & 63];
        out += kTable[(n >> 12) & 63];
        out += (i + 1 < data.size()) ? kTable[(n >> 6) & 63] : '=';
        out += (i + 2 < data.size()) ? kTable[n & 63] : '=';
    }
    return out;
}

bool base64_decode(const std::string& text, std::string& out) {
    auto value = [](char c) -> int {
        if (c >= 'A' && c <= 'Z') return c - 'A';
        if (c >= 'a' && c <= 'z') return c - 'a' + 26;
        if (c >= '0' && c <= '9') return c - '0' + 52;
        if (c == '+') return 62;
        if (c == '/') return 63;
        return -1;
    };
    out.clear();
    int buf = 0, bits = 0;
    for (char c : text) {
        if (c == '=') break;
        int v = value(c);
        if (v < 0) continue;
        buf = (buf << 6) | v;
        bits += 6;
        if (bits >= 8) {
            bits -= 8;
            out += static_cast<char>((buf >> bits) & 0xFF);
        }
    }
    return true;
}

void put_u32(std::string& out, uint32_t v) {
    for (int i = 0; i < 4; ++i) out += static_cast<char>((v >> (8 * i)) & 0xFF);
}
void put_i32(std::string& out, int32_t v) { put_u32(out, static_cast<uint32_t>(v)); }
void put_u16(std::string& out, uint16_t v) {
    out += static_cast<char>(v & 0xFF);
    out += static_cast<char>((v >> 8) & 0xFF);
}

struct Reader {
    const std::string& data;
    size_t at = 0;
    bool u32(uint32_t& v) {
        if (at + 4 > data.size()) return false;
        v = 0;
        for (int i = 0; i < 4; ++i) v |= static_cast<uint32_t>(static_cast<uint8_t>(data[at++])) << (8 * i);
        return true;
    }
    bool i32(int32_t& v) { uint32_t u; if (!u32(u)) return false; v = static_cast<int32_t>(u); return true; }
    bool u16(uint16_t& v) {
        if (at + 2 > data.size()) return false;
        v = static_cast<uint16_t>(static_cast<uint8_t>(data[at]) |
                                  (static_cast<uint16_t>(static_cast<uint8_t>(data[at + 1])) << 8));
        at += 2;
        return true;
    }
    bool u8(uint8_t& v) {
        if (at + 1 > data.size()) return false;
        v = static_cast<uint8_t>(data[at++]);
        return true;
    }
};

} // namespace

std::array<double, 3> voxel_center(int x, int y, int z) {
    return {(x + 0.5) * kVoxelSize, (y + 0.5) * kVoxelSize, (z + 0.5) * kVoxelSize};
}

const Instance* find_terrain(const DataModel& model) {
    for (const auto& child : model.root().children) {
        if (child->class_name != "Workspace") continue;
        for (const auto& sub : child->children)
            if (sub->class_name == "Terrain") return sub.get();
    }
    return nullptr;
}

Instance* find_terrain(DataModel& model) {
    return const_cast<Instance*>(find_terrain(static_cast<const DataModel&>(model)));
}

bool terrain_load(const Instance& terrain, TerrainGrid& out, std::string& error) {
    out.cells.clear();
    auto it = terrain.properties.find("BloxVoxels");
    if (it == terrain.properties.end() || it->second.type != ValueType::BinaryString ||
        it->second.string_value.empty()) {
        return true; // unset = empty terrain
    }
    std::string bytes;
    if (!base64_decode(it->second.string_value, bytes)) {
        error = "Terrain.BloxVoxels is not valid base64";
        return false;
    }
    if (bytes.size() < 8 || bytes.compare(0, 4, "BTR1") != 0) {
        error = "Terrain.BloxVoxels has an unknown format (expected BTR1)";
        return false;
    }
    Reader reader{bytes, 4};
    uint32_t count = 0;
    if (!reader.u32(count)) { error = "Terrain.BloxVoxels is truncated"; return false; }
    for (uint32_t i = 0; i < count; ++i) {
        int32_t x, y, z;
        uint16_t material;
        uint8_t occupancy;
        if (!reader.i32(x) || !reader.i32(y) || !reader.i32(z) || !reader.u16(material) ||
            !reader.u8(occupancy)) {
            error = "Terrain.BloxVoxels is truncated";
            return false;
        }
        if (occupancy > 0) out.cells[{x, y, z}] = {material, occupancy};
    }
    return true;
}

bool terrain_store(DataModel& model, Instance& terrain, const TerrainGrid& grid, std::string& error) {
    std::string bytes = "BTR1";
    put_u32(bytes, static_cast<uint32_t>(grid.cells.size()));
    for (const auto& [key, value] : grid.cells) {
        auto [x, y, z] = key;
        put_i32(bytes, x);
        put_i32(bytes, y);
        put_i32(bytes, z);
        put_u16(bytes, value.first);
        bytes += static_cast<char>(value.second);
    }
    Variant v;
    v.type = ValueType::BinaryString;
    v.string_value = base64_encode(bytes);
    return model.set_property(terrain, "BloxVoxels", v, error);
}

void terrain_fill_block(TerrainGrid& grid, std::array<double, 3> center,
                        std::array<double, 3> size, uint16_t material) {
    auto lo = [&](int axis) {
        return static_cast<int>(std::floor((center[axis] - size[axis] / 2.0) / kVoxelSize));
    };
    auto hi = [&](int axis) {
        // Half-open region: a 32-stud span covers exactly 8 voxels.
        return static_cast<int>(std::ceil((center[axis] + size[axis] / 2.0) / kVoxelSize)) - 1;
    };
    for (int x = lo(0); x <= hi(0); ++x)
        for (int y = lo(1); y <= hi(1); ++y)
            for (int z = lo(2); z <= hi(2); ++z) {
                if (material == 0) grid.cells.erase({x, y, z});
                else grid.cells[{x, y, z}] = {material, 255};
            }
}

void terrain_fill_ball(TerrainGrid& grid, std::array<double, 3> center, double radius,
                       uint16_t material) {
    int r = static_cast<int>(std::ceil(radius / kVoxelSize));
    int cx = static_cast<int>(std::floor(center[0] / kVoxelSize));
    int cy = static_cast<int>(std::floor(center[1] / kVoxelSize));
    int cz = static_cast<int>(std::floor(center[2] / kVoxelSize));
    for (int x = cx - r; x <= cx + r; ++x)
        for (int y = cy - r; y <= cy + r; ++y)
            for (int z = cz - r; z <= cz + r; ++z) {
                auto c = voxel_center(x, y, z);
                double d = std::sqrt((c[0] - center[0]) * (c[0] - center[0]) +
                                     (c[1] - center[1]) * (c[1] - center[1]) +
                                     (c[2] - center[2]) * (c[2] - center[2]));
                if (d > radius) continue;
                if (material == 0) grid.cells.erase({x, y, z});
                else grid.cells[{x, y, z}] = {material, 255};
            }
}

void terrain_fill_cylinder(TerrainGrid& grid, std::array<double, 3> center,
                           double radius, double height, uint16_t material) {
    int lo[3], hi[3];
    double size[3] = {radius * 2, height, radius * 2};
    for (int axis = 0; axis < 3; ++axis) {
        lo[axis] = static_cast<int>(std::floor((center[axis] - size[axis] / 2.0) / kVoxelSize));
        hi[axis] = static_cast<int>(std::ceil((center[axis] + size[axis] / 2.0) / kVoxelSize)) - 1;
    }
    for (int x = lo[0]; x <= hi[0]; ++x)
        for (int y = lo[1]; y <= hi[1]; ++y)
            for (int z = lo[2]; z <= hi[2]; ++z) {
                auto c = voxel_center(x, y, z);
                double dx = c[0] - center[0], dz = c[2] - center[2];
                if (dx * dx + dz * dz > radius * radius) continue;
                if (material == 0) grid.cells.erase({x, y, z});
                else grid.cells[{x, y, z}] = {material, 255};
            }
}

void terrain_fill_wedge(TerrainGrid& grid, std::array<double, 3> center,
                        std::array<double, 3> size, uint16_t material) {
    int lo[3], hi[3];
    for (int axis = 0; axis < 3; ++axis) {
        lo[axis] = static_cast<int>(std::floor((center[axis] - size[axis] / 2.0) / kVoxelSize));
        hi[axis] = static_cast<int>(std::ceil((center[axis] + size[axis] / 2.0) / kVoxelSize)) - 1;
    }
    for (int x = lo[0]; x <= hi[0]; ++x)
        for (int y = lo[1]; y <= hi[1]; ++y)
            for (int z = lo[2]; z <= hi[2]; ++z) {
                auto c = voxel_center(x, y, z);
                double tz = size[2] > 0 ? (c[2] - (center[2] - size[2] / 2.0)) / size[2] : 0.0;
                double top = (center[1] - size[1] / 2.0) + tz * size[1];
                if (c[1] > top) continue;
                if (material == 0) grid.cells.erase({x, y, z});
                else grid.cells[{x, y, z}] = {material, 255};
            }
}

namespace {

void region_bounds(std::array<double, 3> center, std::array<double, 3> size,
                   int lo[3], int hi[3]) {
    for (int axis = 0; axis < 3; ++axis) {
        lo[axis] = static_cast<int>(std::floor((center[axis] - size[axis] / 2.0) / kVoxelSize));
        hi[axis] = static_cast<int>(std::ceil((center[axis] + size[axis] / 2.0) / kVoxelSize)) - 1;
    }
}

} // namespace

void terrain_flatten(TerrainGrid& grid, std::array<double, 3> center,
                     std::array<double, 3> size, double height, uint16_t material) {
    int lo[3], hi[3];
    region_bounds(center, size, lo, hi);
    // Top solid voxel = the highest voxel whose centre is at/below height.
    int top_y = static_cast<int>(std::floor(height / kVoxelSize));
    if (voxel_center(0, top_y, 0)[1] > height) --top_y;
    for (int x = lo[0]; x <= hi[0]; ++x) {
        for (int z = lo[2]; z <= hi[2]; ++z) {
            for (int y = lo[1]; y <= hi[1]; ++y) {
                if (y <= top_y) {
                    if (material != 0) grid.cells[{x, y, z}] = {material, 255};
                } else {
                    grid.cells.erase({x, y, z});
                }
            }
        }
    }
}

void terrain_smooth(TerrainGrid& grid, std::array<double, 3> center,
                    std::array<double, 3> size) {
    int lo[3], hi[3];
    region_bounds(center, size, lo, hi);
    const TerrainGrid before = grid; // pass reads a stable snapshot
    for (int x = lo[0]; x <= hi[0]; ++x) {
        for (int y = lo[1]; y <= hi[1]; ++y) {
            for (int z = lo[2]; z <= hi[2]; ++z) {
                int solid = 0;
                std::map<uint16_t, int> material_votes;
                for (int dx = -1; dx <= 1; ++dx)
                    for (int dy = -1; dy <= 1; ++dy)
                        for (int dz = -1; dz <= 1; ++dz) {
                            if (dx == 0 && dy == 0 && dz == 0) continue;
                            auto it = before.cells.find({x + dx, y + dy, z + dz});
                            if (it != before.cells.end() && it->second.second > 0) {
                                ++solid;
                                ++material_votes[it->second.first];
                            }
                        }
                auto self = before.cells.find({x, y, z});
                bool self_solid = self != before.cells.end() && self->second.second > 0;
                // Majority rule over the 26-neighbourhood; a solid voxel
                // needs only 9 supporters to survive (hysteresis keeps
                // smoothing from eroding thin features immediately).
                bool becomes_solid = self_solid ? solid >= 9 : solid >= 14;
                if (becomes_solid) {
                    uint16_t material = self_solid ? self->second.first : 1280;
                    int best_votes = -1;
                    for (const auto& [mat, votes] : material_votes) {
                        if (votes > best_votes) { best_votes = votes; material = mat; }
                    }
                    grid.cells[{x, y, z}] = {material, 255};
                } else {
                    grid.cells.erase({x, y, z});
                }
            }
        }
    }
}

void terrain_paint(TerrainGrid& grid, std::array<double, 3> center,
                   std::array<double, 3> size, uint16_t material) {
    int lo[3], hi[3];
    region_bounds(center, size, lo, hi);
    for (int x = lo[0]; x <= hi[0]; ++x) {
        for (int y = lo[1]; y <= hi[1]; ++y) {
            for (int z = lo[2]; z <= hi[2]; ++z) {
                auto it = grid.cells.find({x, y, z});
                if (it == grid.cells.end()) continue;
                if (material == 0) grid.cells.erase(it);
                else it->second.first = material;
            }
        }
    }
}

std::array<float, 3> terrain_material_color(uint16_t material) {
    switch (material) {
        case 1280: return {0.35f, 0.62f, 0.25f};  // Grass
        case 1284: return {0.42f, 0.68f, 0.3f};   // LeafyGrass
        case 1312: return {0.85f, 0.76f, 0.55f};  // Sand
        case 800: return {0.45f, 0.45f, 0.48f};   // Slate
        case 816: return {0.6f, 0.6f, 0.6f};      // Concrete
        case 848: return {0.62f, 0.32f, 0.26f};   // Brick
        case 512: return {0.55f, 0.4f, 0.24f};    // Wood
        case 1536: return {0.75f, 0.88f, 0.95f};  // Ice
        case 1328: return {0.92f, 0.94f, 0.97f};  // Snow
        case 2048: return {0.2f, 0.45f, 0.85f};   // Water
        default: return {0.5f, 0.5f, 0.5f};
    }
}

} // namespace blox
