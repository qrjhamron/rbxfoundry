// Mesh asset loading: Roblox .mesh binary formats v1-v4 and SpecialMesh.
// Format layouts follow the public Roblox mesh format (as also implemented
// by the open-source rbx_mesh reference parser). Unsupported versions fail
// with an explicit error; geometry is never faked.
#include "blox/scene.hpp"

#include <algorithm>
#include <cctype>
#include <cstring>
#include <fstream>
#include <sstream>

namespace blox {
namespace {

std::string trim_id(const std::string& id) {
    std::string s = id;
    const std::string prefixes[] = {"rbxassetid://", "rbxasset://", "http://www.roblox.com/asset/?id=",
                                    "https://www.roblox.com/asset/?id="};
    for (const auto& p : prefixes) {
        if (s.rfind(p, 0) == 0) { s = s.substr(p.size()); break; }
    }
    return s;
}

bool is_numeric_id(const std::string& s) {
    return !s.empty() && std::all_of(s.begin(), s.end(), [](char c) { return std::isdigit(static_cast<unsigned char>(c)); });
}

std::string basename_of(const std::string& path) {
    size_t slash = path.find_last_of("/\\");
    return slash == std::string::npos ? path : path.substr(slash + 1);
}

// Resolves an asset id to a local file. Search order: assets dir by id
// ("<id>.mesh" / "<id>.png"), assets dir by file name, place dir by name.
// Roblox login is never required; online-only ids simply won't resolve.
std::string resolve_asset(const std::string& raw_id, const std::string& extension,
                          const SceneOptions& options, bool& found) {
    found = false;
    std::string id = trim_id(raw_id);
    std::vector<std::string> candidates;
    if (is_numeric_id(id)) {
        if (!options.assets_dir.empty()) {
            candidates.push_back(options.assets_dir + "/" + id + extension);
            candidates.push_back(options.assets_dir + "/" + id);
        }
    } else {
        std::string base = basename_of(id);
        if (!options.assets_dir.empty()) candidates.push_back(options.assets_dir + "/" + base);
        if (!options.place_dir.empty()) candidates.push_back(options.place_dir + "/" + base);
        if (!options.place_dir.empty()) candidates.push_back(options.place_dir + "/" + id);
        candidates.push_back(id);
    }
    for (const auto& path : candidates) {
        std::ifstream test(path, std::ios::binary);
        if (test.good()) { found = true; return path; }
    }
    return "";
}

std::vector<uint8_t> read_file(const std::string& path, bool& ok) {
    std::ifstream in(path, std::ios::binary | std::ios::ate);
    if (!in) { ok = false; return {}; }
    std::streamsize size = in.tellg();
    in.seekg(0);
    std::vector<uint8_t> data(static_cast<size_t>(size));
    ok = in.read(reinterpret_cast<char*>(data.data()), size).good();
    return data;
}

struct Reader {
    const uint8_t* data;
    size_t size;
    size_t pos = 0;
    bool u8(uint8_t& v) { if (pos + 1 > size) return false; v = data[pos++]; return true; }
    bool u16(uint16_t& v) {
        if (pos + 2 > size) return false;
        std::memcpy(&v, data + pos, 2); pos += 2; return true;
    }
    bool u32(uint32_t& v) {
        if (pos + 4 > size) return false;
        std::memcpy(&v, data + pos, 4); pos += 4; return true;
    }
    bool f32(float& v) {
        if (pos + 4 > size) return false;
        std::memcpy(&v, data + pos, 4); pos += 4; return true;
    }
    bool skip(size_t n) { if (pos + n > size) return false; pos += n; return true; }
    bool bytes(size_t n) { return pos + n <= size; }
};

struct MeshData {
    bool skinned = false;
    uint32_t bone_count = 0;
    std::vector<std::array<float, 3>> positions;
    std::vector<std::array<float, 3>> normals;
    std::vector<std::array<float, 2>> uvs;
    std::vector<uint32_t> indices;
};

// Parses vertex block shared by v2/v3/v4 (vertex stride 36 truncated or 40 full).
bool parse_vertices_v2(Reader& r, uint32_t count, uint8_t stride, MeshData& out, std::string& error) {
    if (stride != 36 && stride != 40) { error = "unsupported mesh vertex size " + std::to_string(stride); return false; }
    for (uint32_t i = 0; i < count; ++i) {
        std::array<float, 3> pos, norm;
        std::array<float, 2> uv;
        for (float& f : pos) if (!r.f32(f)) { error = "truncated mesh vertex data"; return false; }
        for (float& f : norm) if (!r.f32(f)) { error = "truncated mesh vertex data"; return false; }
        for (float& f : uv) if (!r.f32(f)) { error = "truncated mesh vertex data"; return false; }
        // tangent i8[4] (+ colour u8[4] when stride 40) — not needed for rendering.
        if (!r.skip(stride - 32)) { error = "truncated mesh vertex data"; return false; }
        out.positions.push_back(pos);
        out.normals.push_back(norm);
        out.uvs.push_back({uv[0], 1.0f - uv[1]}); // Roblox UV origin is bottom-left
    }
    return true;
}

bool parse_faces(Reader& r, uint32_t count, MeshData& out, std::string& error) {
    for (uint32_t i = 0; i < count; ++i) {
        for (int k = 0; k < 3; ++k) {
            uint32_t idx;
            if (!r.u32(idx)) { error = "truncated mesh face data"; return false; }
            out.indices.push_back(idx);
        }
    }
    return true;
}

bool parse_mesh(const std::vector<uint8_t>& data, MeshData& out, std::string& error) {
    if (data.size() < 13) { error = "mesh file too small"; return false; }
    std::string header(reinterpret_cast<const char*>(data.data()), std::min<size_t>(data.size(), 64));
    size_t nl = header.find('\n');
    if (nl == std::string::npos) { error = "mesh file has no version header"; return false; }
    std::string version = header.substr(0, nl);

    if (version == "version 1.00" || version == "version 1.01") {
        // Text format: line 2 = face count, then one [pos][normal][uv] triple per vertex.
        std::string text(reinterpret_cast<const char*>(data.data()), data.size());
        std::istringstream stream(text);
        std::string line;
        std::getline(stream, line); // version
        if (!std::getline(stream, line)) { error = "v1 mesh missing face count"; return false; }
        uint32_t faces = static_cast<uint32_t>(std::strtoul(line.c_str(), nullptr, 10));
        std::string rest((std::istreambuf_iterator<char>(stream)), std::istreambuf_iterator<char>());
        // Extract bracketed float triples.
        std::vector<std::array<float, 3>> triples;
        size_t at = 0;
        while (true) {
            size_t open = rest.find('[', at);
            if (open == std::string::npos) break;
            size_t close = rest.find(']', open);
            if (close == std::string::npos) break;
            std::string inside = rest.substr(open + 1, close - open - 1);
            std::array<float, 3> triple{};
            int matched = std::sscanf(inside.c_str(), "%f,%f,%f", &triple[0], &triple[1], &triple[2]);
            if (matched != 3) { error = "v1 mesh has a malformed vertex"; return false; }
            triples.push_back(triple);
            at = close + 1;
        }
        if (triples.size() < static_cast<size_t>(faces) * 3) { error = "v1 mesh vertex count mismatch"; return false; }
        // Triples are grouped [pos][normal][uv] per vertex, in face order.
        for (size_t v = 0; v + 2 < triples.size() && out.positions.size() < static_cast<size_t>(faces) * 3; v += 3) {
            out.positions.push_back(triples[v]);
            out.normals.push_back(triples[v + 1]);
            out.uvs.push_back({triples[v + 2][0], 1.0f - triples[v + 2][1]});
            out.indices.push_back(static_cast<uint32_t>(out.indices.size()));
        }
        return true;
    }

    Reader r{data.data(), data.size(), nl + 1};
    if (version == "version 2.00") {
        uint16_t header_size;
        uint8_t vertex_stride, face_stride;
        uint32_t vcount, fcount;
        if (!r.u16(header_size) || !r.u8(vertex_stride) || !r.u8(face_stride) ||
            !r.u32(vcount) || !r.u32(fcount)) { error = "truncated v2 mesh header"; return false; }
        if (header_size != 12) { error = "unexpected v2 mesh header size"; return false; }
        return parse_vertices_v2(r, vcount, vertex_stride, out, error) && parse_faces(r, fcount, out, error);
    }
    if (version == "version 3.00" || version == "version 3.01") {
        uint16_t header_size, lod_stride;
        uint8_t vertex_stride, face_stride;
        uint16_t lod_count;
        uint32_t vcount, fcount;
        if (!r.u16(header_size) || !r.u8(vertex_stride) || !r.u8(face_stride) ||
            !r.u16(lod_stride) || !r.u16(lod_count) || !r.u32(vcount) || !r.u32(fcount)) {
            error = "truncated v3 mesh header"; return false;
        }
        return parse_vertices_v2(r, vcount, vertex_stride, out, error) && parse_faces(r, fcount, out, error);
        // LOD table follows the faces; LOD 0 is the full range, which we render.
    }
    if (version == "version 4.00" || version == "version 4.01") {
        uint16_t header_size;
        uint16_t lod_type;
        uint32_t vcount, fcount;
        uint16_t lod_count, bone_count;
        uint32_t bone_names_len;
        uint16_t subset_count;
        uint8_t lod_hq, padding;
        if (!r.u16(header_size) || !r.u16(lod_type) || !r.u32(vcount) || !r.u32(fcount) ||
            !r.u16(lod_count) || !r.u16(bone_count) || !r.u32(bone_names_len) ||
            !r.u16(subset_count) || !r.u8(lod_hq) || !r.u8(padding)) {
            error = "truncated v4 mesh header"; return false;
        }
        // v4 vertices are always the full 40-byte form.
        if (!parse_vertices_v2(r, vcount, 40, out, error)) return false;
        if (bone_count > 0) {
            if (!r.skip(static_cast<size_t>(vcount) * 8)) { error = "truncated v4 envelopes"; return false; }
        }
        if (bone_count > 0) { out.skinned = true; out.bone_count = bone_count; }
        return parse_faces(r, fcount, out, error);
    }
    if (version == "version 5.00") {
        // Layout per the open-source rbx_mesh reference parser (there is
        // no official public spec): 32-byte header, full 40-byte
        // vertices, then envelopes (8B/vertex when bones exist), faces,
        // LODs, bones, bone names, subsets and a FACS trailer. We parse
        // the render geometry only; skinning data is skipped and the
        // caller warns that the mesh renders unskinned.
        uint16_t header_size, lod_type, lod_count, bone_count, subset_count;
        uint32_t vcount, fcount, bone_names_len, facs_format, sizeof_facs;
        uint8_t lod_hq, padding;
        if (!r.u16(header_size) || !r.u16(lod_type) || !r.u32(vcount) || !r.u32(fcount) ||
            !r.u16(lod_count) || !r.u16(bone_count) || !r.u32(bone_names_len) ||
            !r.u16(subset_count) || !r.u8(lod_hq) || !r.u8(padding) ||
            !r.u32(facs_format) || !r.u32(sizeof_facs)) {
            error = "truncated v5 mesh header"; return false;
        }
        if (header_size != 32) { error = "unexpected v5 mesh header size"; return false; }
        if (!parse_vertices_v2(r, vcount, 40, out, error)) return false;
        if (bone_count > 0) {
            out.skinned = true;
            out.bone_count = bone_count;
            if (!r.skip(static_cast<size_t>(vcount) * 8)) { error = "truncated v5 envelopes"; return false; }
        }
        return parse_faces(r, fcount, out, error);
    }
    error = "unsupported Roblox mesh version '" + version +
            "' (supported: 1.00, 1.01, 2.00, 3.00, 3.01, 4.00, 4.01, 5.00)";
    return false;
}

const Variant* get_prop(const Instance& inst, const char* name) {
    auto it = inst.properties.find(name);
    return it == inst.properties.end() ? nullptr : &it->second;
}
std::string get_string(const Instance& inst, const char* name) {
    const Variant* v = get_prop(inst, name);
    if (v && (v->type == ValueType::String || v->type == ValueType::ContentId)) return v->string_value;
    return "";
}
int64_t get_enum(const Instance& inst, const char* name, int64_t fallback) {
    const Variant* v = get_prop(inst, name);
    if (v && (v->type == ValueType::Enum || v->type == ValueType::Int32 || v->type == ValueType::Int64))
        return v->int_value;
    return fallback;
}
std::array<double, 3> get_vec3(const Instance& inst, const char* name, std::array<double, 3> fallback) {
    const Variant* v = get_prop(inst, name);
    if (v && v->type == ValueType::Vector3) return v->vec3;
    return fallback;
}

// Scales parsed mesh geometry into `out`. When `fit_size` is set (MeshPart),
// the mesh bounding box is scaled per-axis to the part Size, matching how
// Studio displays MeshParts. Otherwise vertices are scaled by `scale`.
void emit_mesh(const MeshData& mesh, const std::array<double, 3>* fit_size,
               const std::array<double, 3>& scale, DrawItem& out) {
    std::array<double, 3> factor = scale;
    if (fit_size) {
        std::array<double, 3> mn{1e30, 1e30, 1e30}, mx{-1e30, -1e30, -1e30};
        for (const auto& p : mesh.positions)
            for (int i = 0; i < 3; ++i) { mn[i] = std::min(mn[i], static_cast<double>(p[i])); mx[i] = std::max(mx[i], static_cast<double>(p[i])); }
        for (int i = 0; i < 3; ++i) {
            double extent = mx[i] - mn[i];
            factor[i] = extent > 1e-9 ? (*fit_size)[i] / extent : 1.0;
        }
        // Re-centre on the bounding-box centre.
        std::array<double, 3> centre{(mn[0] + mx[0]) / 2, (mn[1] + mx[1]) / 2, (mn[2] + mx[2]) / 2};
        for (size_t i = 0; i < mesh.positions.size(); ++i) {
            out.positions.push_back({static_cast<float>((mesh.positions[i][0] - centre[0]) * factor[0]),
                                     static_cast<float>((mesh.positions[i][1] - centre[1]) * factor[1]),
                                     static_cast<float>((mesh.positions[i][2] - centre[2]) * factor[2])});
            out.normals.push_back(mesh.normals[i]);
            out.uvs.push_back(mesh.uvs[i]);
        }
        out.indices = mesh.indices;
        return;
    }
    for (size_t i = 0; i < mesh.positions.size(); ++i) {
        out.positions.push_back({static_cast<float>(mesh.positions[i][0] * factor[0]),
                                 static_cast<float>(mesh.positions[i][1] * factor[1]),
                                 static_cast<float>(mesh.positions[i][2] * factor[2])});
        out.normals.push_back(mesh.normals[i]);
        out.uvs.push_back(mesh.uvs[i]);
    }
    out.indices = mesh.indices;
}

} // namespace

bool load_mesh_geometry(const std::string& mesh_id, const std::string& texture_id,
                        const SceneOptions& options, const Instance& part,
                        DrawItem& out, std::string& error) {
    std::string effective_mesh_id = mesh_id;
    std::string effective_texture_id = texture_id;
    const Instance* special = nullptr;
    for (const auto& child : part.children) {
        if (child->class_name == "SpecialMesh") { special = child.get(); break; }
    }

    auto size = get_vec3(part, "Size", {4, 1, 2});
    if (special) {
        // SpecialMesh.MeshType: Head=0 Torso=1 Wedge=2 Sphere=3 Cylinder=4
        // FileMesh=5 Brick=6 Prism=7 Pyramid=8 ParallelRamp=9
        // RightAngleRamp=10 Corner1=11 Corner2=12.
        int64_t mesh_type = get_enum(*special, "MeshType", 5);
        auto scale = get_vec3(*special, "Scale", {1, 1, 1});
        std::array<double, 3> scaled{size[0] * scale[0], size[1] * scale[1], size[2] * scale[2]};
        if (mesh_type != 5) {
            switch (mesh_type) {
                case 0: // Head: sphere approximation (Roblox head is a rounded form)
                case 3: gen_sphere(scaled, 24, out); break;
                case 4: gen_cylinder(scaled, 24, out); break;
                case 2: case 9: case 10: gen_wedge(scaled, out); break;
                case 11: case 12: gen_corner_wedge(scaled, out); break;
                case 8: gen_wedge(scaled, out); break; // Pyramid approximated by wedge (documented)
                default: gen_box(scaled, out); break; // Torso, Brick, Prism approximation
            }
            std::string tex = get_string(*special, "TextureId");
            if (!tex.empty()) effective_texture_id = tex;
            if (!effective_texture_id.empty()) {
                bool found = false;
                std::string path = resolve_asset(effective_texture_id, ".png", options, found);
                if (found) out.texture_path = path;
            }
            return true;
        }
        effective_mesh_id = get_string(*special, "MeshId");
        std::string tex = get_string(*special, "TextureId");
        if (!tex.empty()) effective_texture_id = tex;
        if (effective_mesh_id.empty()) { error = "SpecialMesh (FileMesh) has no MeshId"; return false; }
        bool found = false;
        std::string path = resolve_asset(effective_mesh_id, ".mesh", options, found);
        if (!found) {
            error = "mesh asset '" + effective_mesh_id + "' not found locally (online assets need a local copy in the assets directory; nothing was substituted)";
            return false;
        }
        bool ok = false;
        auto bytes = read_file(path, ok);
        if (!ok) { error = "cannot read mesh file '" + path + "'"; return false; }
        MeshData mesh;
        if (!parse_mesh(bytes, mesh, error)) { error = "mesh '" + path + "': " + error; return false; }
        auto mesh_scale = get_vec3(*special, "Scale", {1, 1, 1});
        emit_mesh(mesh, nullptr, mesh_scale, out);
        if (mesh.skinned) {
            out.warning = "skinned mesh (" + std::to_string(mesh.bone_count) +
                          " bones) rendered without skinning; skinning is unsupported";
        }
        if (!effective_texture_id.empty()) {
            bool tfound = false;
            std::string tpath = resolve_asset(effective_texture_id, ".png", options, tfound);
            if (tfound) out.texture_path = tpath;
        }
        return true;
    }

    // Plain MeshPart.
    if (effective_mesh_id.empty()) {
        error = "MeshPart has no MeshId (no geometry to render; nothing was substituted)";
        return false;
    }
    bool found = false;
    std::string path = resolve_asset(effective_mesh_id, ".mesh", options, found);
    if (!found) {
        error = "mesh asset '" + effective_mesh_id + "' not found locally (online assets need a local copy in the assets directory; nothing was substituted)";
        return false;
    }
    bool ok = false;
    auto bytes = read_file(path, ok);
    if (!ok) { error = "cannot read mesh file '" + path + "'"; return false; }
    MeshData mesh;
    if (!parse_mesh(bytes, mesh, error)) { error = "mesh '" + path + "': " + error; return false; }
    emit_mesh(mesh, &size, {1, 1, 1}, out);
    if (mesh.skinned) {
        out.warning = "skinned mesh (" + std::to_string(mesh.bone_count) +
                      " bones) rendered without skinning; skinning is unsupported";
    }
    if (!effective_texture_id.empty()) {
        bool tfound = false;
        std::string tpath = resolve_asset(effective_texture_id, ".png", options, tfound);
        if (tfound) out.texture_path = tpath;
        // A missing texture is reported by build_scene via missing_assets,
        // but the geometry still renders untextured.
    }
    return true;
}

std::string resolve_texture_asset(const std::string& raw_id, const SceneOptions& options, bool& found) {
    return resolve_asset(raw_id, ".png", options, found);
}

} // namespace blox
