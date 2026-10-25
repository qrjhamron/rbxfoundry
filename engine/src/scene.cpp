#include "blox/scene.hpp"
#include "blox/brickcolor.hpp"
#include "blox/terrain.hpp"

#include <filesystem>

#include "blox/ui.hpp"
#include <set>

#include <algorithm>
#include <cmath>
#include <map>

namespace blox {
namespace {

using V3 = std::array<float, 3>;

V3 v3(double x, double y, double z) {
    return {static_cast<float>(x), static_cast<float>(y), static_cast<float>(z)};
}
V3 sub(const V3& a, const V3& b) { return {a[0] - b[0], a[1] - b[1], a[2] - b[2]}; }
V3 cross(const V3& a, const V3& b) {
    return {a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]};
}
V3 normalize(V3 a) {
    float len = std::sqrt(a[0] * a[0] + a[1] * a[1] + a[2] * a[2]);
    if (len <= 0) return {0, 1, 0};
    return {a[0] / len, a[1] / len, a[2] / len};
}

// Appends a flat-shaded quad (two triangles, CCW seen from the normal side).
// Orients a face normal away from the local origin (all part shapes here
// are convex and centred), independent of vertex winding order.
V3 outward(const V3& n, const V3& centroid) {
    float d = n[0] * centroid[0] + n[1] * centroid[1] + n[2] * centroid[2];
    return d < 0 ? V3{-n[0], -n[1], -n[2]} : n;
}

void quad(DrawItem& out, const V3& a, const V3& b, const V3& c, const V3& d) {
    V3 centroid{(a[0] + b[0] + c[0] + d[0]) / 4, (a[1] + b[1] + c[1] + d[1]) / 4,
                (a[2] + b[2] + c[2] + d[2]) / 4};
    V3 n = outward(normalize(cross(sub(d, a), sub(b, a))), centroid);
    uint32_t base = static_cast<uint32_t>(out.positions.size());
    const V3 verts[4] = {a, b, c, d};
    const float uv[4][2] = {{0, 0}, {1, 0}, {1, 1}, {0, 1}};
    for (int i = 0; i < 4; ++i) {
        out.positions.push_back(verts[i]);
        out.normals.push_back(n);
        out.uvs.push_back({uv[i][0], uv[i][1]});
    }
    out.indices.insert(out.indices.end(), {base, base + 1, base + 2, base, base + 2, base + 3});
}

void tri(DrawItem& out, const V3& a, const V3& b, const V3& c) {
    V3 centroid{(a[0] + b[0] + c[0]) / 3, (a[1] + b[1] + c[1]) / 3, (a[2] + b[2] + c[2]) / 3};
    V3 n = outward(normalize(cross(sub(b, a), sub(c, a))), centroid);
    uint32_t base = static_cast<uint32_t>(out.positions.size());
    out.positions.insert(out.positions.end(), {a, b, c});
    out.normals.insert(out.normals.end(), {n, n, n});
    out.uvs.insert(out.uvs.end(), {{0, 0}, {1, 0}, {0, 1}});
    out.indices.insert(out.indices.end(), {base, base + 1, base + 2});
}

const Variant* prop(const Instance& inst, const char* name) {
    auto it = inst.properties.find(name);
    return it == inst.properties.end() ? nullptr : &it->second;
}
std::array<double, 3> prop_vec3(const Instance& inst, const char* name, std::array<double, 3> fallback) {
    const Variant* v = prop(inst, name);
    if (v && v->type == ValueType::Vector3) return v->vec3;
    return fallback;
}
double prop_float(const Instance& inst, const char* name, double fallback) {
    const Variant* v = prop(inst, name);
    if (!v) return fallback;
    if (v->type == ValueType::Float32 || v->type == ValueType::Float64) return v->float_value;
    if (v->type == ValueType::Int32 || v->type == ValueType::Int64) return static_cast<double>(v->int_value);
    return fallback;
}
bool prop_bool(const Instance& inst, const char* name, bool fallback) {
    const Variant* v = prop(inst, name);
    return (v && v->type == ValueType::Bool) ? v->bool_value : fallback;
}
int64_t prop_enum(const Instance& inst, const char* name, int64_t fallback) {
    const Variant* v = prop(inst, name);
    if (!v) return fallback;
    if (v->type == ValueType::Enum || v->type == ValueType::Int32 || v->type == ValueType::Int64) return v->int_value;
    return fallback;
}
std::string prop_string(const Instance& inst, const char* name) {
    const Variant* v = prop(inst, name);
    if (v && (v->type == ValueType::String || v->type == ValueType::ContentId)) return v->string_value;
    return "";
}

// BrickColor palette shared with the DataModel (brickcolor.hpp).
std::array<float, 3> part_color_srgb(const Instance& inst) {
    const Variant* c = prop(inst, "Color");
    if (c && c->type == ValueType::Color3)
        return {static_cast<float>(c->vec3[0]), static_cast<float>(c->vec3[1]), static_cast<float>(c->vec3[2])};
    const Variant* b = prop(inst, "BrickColor");
    if (b && b->type == ValueType::BrickColor) {
        auto rgb = brick_color_rgb(b->int_value);
        return {rgb[0] / 255.0f, rgb[1] / 255.0f, rgb[2] / 255.0f};
    }
    return {0.64f, 0.64f, 0.65f};
}

void set_model_from_cframe(DrawItem& draw, const Instance& inst) {
    Variant identity = Variant::make_cframe(0, 0, 0);
    const Variant* cf = prop(inst, "CFrame");
    const Variant& v = (cf && cf->type == ValueType::CFrame) ? *cf : identity;
    // Column-major: basis axes are the rotation columns.
    const auto& r = v.rotation; // row-major rows
    draw.model = {
        static_cast<float>(r[0]), static_cast<float>(r[3]), static_cast<float>(r[6]), 0,
        static_cast<float>(r[1]), static_cast<float>(r[4]), static_cast<float>(r[7]), 0,
        static_cast<float>(r[2]), static_cast<float>(r[5]), static_cast<float>(r[8]), 0,
        static_cast<float>(v.position[0]), static_cast<float>(v.position[1]),
        static_cast<float>(v.position[2]), 1,
    };
}

const Instance* find_child_of_root(const DataModel& model, const std::string& class_name) {
    for (const auto& child : model.root().children)
        if (child->class_name == class_name) return child.get();
    return nullptr;
}

} // namespace

void gen_box(const std::array<double, 3>& size, DrawItem& out) {
    double hx = size[0] / 2, hy = size[1] / 2, hz = size[2] / 2;
    V3 nnn = v3(-hx, -hy, -hz), pnn = v3(hx, -hy, -hz), ppn = v3(hx, hy, -hz), npn = v3(-hx, hy, -hz);
    V3 nnp = v3(-hx, -hy, hz), pnp = v3(hx, -hy, hz), ppp = v3(hx, hy, hz), npp = v3(-hx, hy, hz);
    quad(out, pnp, nnp, npp, ppp); // +Z
    quad(out, nnn, pnn, ppn, npn); // -Z
    quad(out, pnn, pnp, ppp, ppn); // +X
    quad(out, nnp, nnn, npn, npp); // -X
    quad(out, npn, ppn, ppp, npp); // +Y
    quad(out, nnn, pnn, pnp, nnp); // -Y
}

void gen_sphere(const std::array<double, 3>& size, uint32_t segments, DrawItem& out) {
    double radius = std::min({size[0], size[1], size[2]}) / 2.0;
    uint32_t rings = std::max<uint32_t>(3, segments / 2);
    uint32_t base = static_cast<uint32_t>(out.positions.size());
    const double pi = 3.14159265358979323846;
    for (uint32_t ring = 0; ring <= rings; ++ring) {
        double phi = pi * static_cast<double>(ring) / rings; // 0..pi from +Y
        for (uint32_t seg = 0; seg <= segments; ++seg) {
            double theta = 2 * pi * static_cast<double>(seg) / segments;
            V3 dir = v3(std::sin(phi) * std::cos(theta), std::cos(phi), std::sin(phi) * std::sin(theta));
            out.positions.push_back(v3(dir[0] * radius, dir[1] * radius, dir[2] * radius));
            out.normals.push_back(dir);
            out.uvs.push_back({static_cast<float>(seg) / segments, static_cast<float>(ring) / rings});
        }
    }
    for (uint32_t ring = 0; ring < rings; ++ring) {
        for (uint32_t seg = 0; seg < segments; ++seg) {
            uint32_t a = base + ring * (segments + 1) + seg;
            uint32_t b = a + segments + 1;
            out.indices.insert(out.indices.end(), {a, b, a + 1, b, b + 1, a + 1});
        }
    }
}

void gen_cylinder(const std::array<double, 3>& size, uint32_t segments, DrawItem& out) {
    // Axis along X (Roblox cylinder caps face +-X).
    double hx = size[0] / 2;
    double radius = std::min(size[1], size[2]) / 2.0;
    const double pi = 3.14159265358979323846;
    uint32_t side_base = static_cast<uint32_t>(out.positions.size());
    for (uint32_t seg = 0; seg <= segments; ++seg) {
        double theta = 2 * pi * static_cast<double>(seg) / segments;
        float y = static_cast<float>(std::cos(theta) * radius);
        float z = static_cast<float>(std::sin(theta) * radius);
        V3 n = normalize(v3(0, y, z));
        out.positions.push_back(v3(-hx, y, z));
        out.normals.push_back(n);
        out.uvs.push_back({0, static_cast<float>(seg) / segments});
        out.positions.push_back(v3(hx, y, z));
        out.normals.push_back(n);
        out.uvs.push_back({1, static_cast<float>(seg) / segments});
    }
    for (uint32_t seg = 0; seg < segments; ++seg) {
        uint32_t a = side_base + seg * 2;
        out.indices.insert(out.indices.end(), {a, a + 1, a + 2, a + 1, a + 3, a + 2});
    }
    // Caps as triangle fans.
    for (int side = 0; side < 2; ++side) {
        double x = side == 0 ? -hx : hx;
        V3 n = v3(side == 0 ? -1 : 1, 0, 0);
        uint32_t centre = static_cast<uint32_t>(out.positions.size());
        out.positions.push_back(v3(x, 0, 0));
        out.normals.push_back(n);
        out.uvs.push_back({0.5f, 0.5f});
        uint32_t ring_start = static_cast<uint32_t>(out.positions.size());
        for (uint32_t seg = 0; seg <= segments; ++seg) {
            double theta = 2 * pi * static_cast<double>(seg) / segments;
            float y = static_cast<float>(std::cos(theta) * radius);
            float z = static_cast<float>(std::sin(theta) * radius);
            out.positions.push_back(v3(x, y, z));
            out.normals.push_back(n);
            out.uvs.push_back({y / static_cast<float>(radius) * 0.5f + 0.5f, z / static_cast<float>(radius) * 0.5f + 0.5f});
        }
        for (uint32_t seg = 0; seg < segments; ++seg) {
            if (side == 0)
                out.indices.insert(out.indices.end(), {centre, ring_start + seg + 1, ring_start + seg});
            else
                out.indices.insert(out.indices.end(), {centre, ring_start + seg, ring_start + seg + 1});
        }
    }
}

void gen_wedge(const std::array<double, 3>& size, DrawItem& out) {
    // Roblox WedgePart: the tall vertical face is the Back (-Z) and the
    // sloped face is the Front (+Z), rising from the bottom-front edge to
    // the top-back edge.
    double hx = size[0] / 2, hy = size[1] / 2, hz = size[2] / 2;
    V3 a = v3(-hx, -hy, hz), b = v3(hx, -hy, hz), c = v3(hx, -hy, -hz), d = v3(-hx, -hy, -hz);
    V3 e = v3(-hx, hy, -hz), f = v3(hx, hy, -hz);
    quad(out, a, d, c, b); // bottom
    quad(out, d, c, f, e); // back -Z (full height)
    quad(out, b, a, e, f); // slope (Front surface)
    tri(out, a, d, e);     // left
    tri(out, b, f, c);     // right
}

void gen_corner_wedge(const std::array<double, 3>& size, DrawItem& out) {
    // Roblox CornerWedgePart: single high corner at the back-left; two
    // slopes face Front (+Z) and Right (+X), the Back/Left faces are
    // triangles meeting at the peak.
    double hx = size[0] / 2, hy = size[1] / 2, hz = size[2] / 2;
    V3 a = v3(-hx, -hy, hz), b = v3(hx, -hy, hz), c = v3(hx, -hy, -hz), d = v3(-hx, -hy, -hz);
    V3 e = v3(-hx, hy, -hz); // peak above the back-left corner
    quad(out, a, d, c, b); // bottom
    tri(out, a, b, e);     // front slope
    tri(out, b, c, e);     // right slope
    tri(out, c, d, e);     // back triangle
    tri(out, d, a, e);     // left triangle
}

namespace {

void extract_part(const Instance& inst, const SceneOptions& options, Scene& scene) {
    const std::string& cls = inst.class_name;
    DrawItem draw;
    draw.name = inst.name;
    auto size = prop_vec3(inst, "Size", {4, 1, 2});
    std::string error;

    if (cls == "Part") {
        int64_t shape = prop_enum(inst, "Shape", 1);
        if (shape == 0) gen_sphere(size, options.sphere_segments, draw);
        else if (shape == 2) gen_cylinder(size, options.cylinder_segments, draw);
        else gen_box(size, draw);
    } else if (cls == "WedgePart") {
        gen_wedge(size, draw);
    } else if (cls == "CornerWedgePart") {
        gen_corner_wedge(size, draw);
    } else if (cls == "MeshPart") {
        std::string mesh_id = prop_string(inst, "MeshId");
        std::string texture_id = prop_string(inst, "TextureID");
        // SurfaceAppearance PBR maps (Phase 11).
        std::string normal_id, roughness_id, metalness_id;
        for (const auto& child : inst.children) {
            if (child->class_name == "SurfaceAppearance") {
                std::string color_map = prop_string(*child, "ColorMap");
                if (!color_map.empty()) texture_id = color_map;
                normal_id = prop_string(*child, "NormalMap");
                roughness_id = prop_string(*child, "RoughnessMap");
                metalness_id = prop_string(*child, "MetalnessMap");
            }
        }
        if (!load_mesh_geometry(mesh_id, texture_id, options, inst, draw, error)) {
            scene.skipped.push_back("MeshPart '" + inst.name + "': " + error);
            if (!mesh_id.empty()) scene.missing_assets.push_back(mesh_id);
            return;
        }
        if (!texture_id.empty() && draw.texture_path.empty()) {
            scene.missing_assets.push_back(texture_id + " (texture)");
        }
        auto apply_map = [&](const std::string& id, std::string& slot, const char* label) {
            if (id.empty()) return;
            bool found = false;
            std::string path = resolve_texture_asset(id, options, found);
            if (found) slot = path;
            else scene.missing_assets.push_back(id + " (" + label + ")");
        };
        apply_map(normal_id, draw.normal_path, "normal map");
        apply_map(roughness_id, draw.roughness_path, "roughness map");
        apply_map(metalness_id, draw.metalness_path, "metalness map");
        scene.part_count++;
        // Mesh geometry replaces Size-based geometry; CFrame/colour below.
        draw.color = srgb_to_linear(part_color_srgb(inst));
        draw.transparency = static_cast<float>(prop_float(inst, "Transparency", 0.0));
        draw.material = static_cast<uint32_t>(prop_enum(inst, "Material", 256));
        draw.cast_shadow = prop_bool(inst, "CastShadow", true);
        set_model_from_cframe(draw, inst);
        scene.draws.push_back(std::move(draw));
        return;
    } else if (cls == "SpawnLocation" || cls == "Seat" || cls == "TrussPart") {
        gen_box(size, draw);
    } else {
        // UnionOperation, NegateOperation and other BaseParts have no
        // representable geometry yet (CSG). Report, never fake.
        scene.skipped.push_back(cls + " '" + inst.name + "': geometry type not supported yet (no CSG)");
        return;
    }

    double transparency = prop_float(inst, "Transparency", 0.0);
    if (transparency >= 1.0) {
        scene.skipped.push_back(cls + " '" + inst.name + "': fully transparent (Transparency=1)");
        return;
    }
    scene.part_count++;
    draw.color = srgb_to_linear(part_color_srgb(inst));
    draw.transparency = static_cast<float>(transparency);
    draw.material = static_cast<uint32_t>(prop_enum(inst, "Material", 256));
    draw.cast_shadow = prop_bool(inst, "CastShadow", true);
    set_model_from_cframe(draw, inst);

    scene.draws.push_back(std::move(draw));

    // SurfaceGui children: rasterize the GUI onto the part's face as a
    // texture and add a textured face quad (Phase 4). The texture is a
    // generated PNG in the place directory, removed never (documented).
    for (const auto& child : inst.children) {
        if (child->class_name != "SurfaceGui") continue;
        const Instance& surface = *child;
        if (!prop_bool(surface, "Enabled", true)) continue;
        int64_t face = prop_enum(surface, "Face", 5); // NormalId, Front=5
        // u/v axes (cross(u,v) == outward normal) and half extents per face.
        std::array<double, 3> u_axis{1, 0, 0}, v_axis{0, 1, 0}, normal{0, 0, 1};
        double hu = size[0] / 2, hv = size[1] / 2;
        switch (face) {
            case 0: u_axis = {0, 0, -1}; v_axis = {0, 1, 0}; normal = {1, 0, 0}; hu = size[2] / 2; hv = size[1] / 2; break; // Right
            case 1: u_axis = {1, 0, 0}; v_axis = {0, 0, -1}; normal = {0, 1, 0}; hu = size[0] / 2; hv = size[2] / 2; break; // Top
            case 2: u_axis = {-1, 0, 0}; v_axis = {0, 1, 0}; normal = {0, 0, -1}; hu = size[0] / 2; hv = size[1] / 2; break; // Back
            case 3: u_axis = {0, 0, 1}; v_axis = {0, 1, 0}; normal = {-1, 0, 0}; hu = size[2] / 2; hv = size[1] / 2; break; // Left
            case 4: u_axis = {1, 0, 0}; v_axis = {0, 0, 1}; normal = {0, -1, 0}; hu = size[0] / 2; hv = size[2] / 2; break; // Bottom
            default: break; // Front
        }
        double pps = prop_float(surface, "PixelsPerStud", 50.0);
        if (pps <= 0) pps = 50.0;
        uint32_t tex_w = static_cast<uint32_t>(std::clamp(hu * 2 * pps, 8.0, 1024.0));
        uint32_t tex_h = static_cast<uint32_t>(std::clamp(hv * 2 * pps, 8.0, 1024.0));
        UiOptions ui_options;
        ui_options.width = tex_w;
        ui_options.height = tex_h;
        ui_options.assets_dir = options.assets_dir;
        ui_options.place_dir = options.place_dir;
        UiScene sub = build_ui_layout(surface, 0, 0, static_cast<float>(tex_w),
                                      static_cast<float>(tex_h), ui_options, surface.name);
        auto pixels = rasterize_ui(sub, tex_w, tex_h, {0, 0, 0, 0});
        static int surface_counter = 0;
        std::string dir = options.place_dir.empty() ? "." : options.place_dir;
        std::string tex_path = dir + "/.blox-surface-" + std::to_string(++surface_counter) + ".png";
        std::string png_error;
        if (!write_png(tex_path, tex_w, tex_h, pixels, png_error)) {
            scene.skipped.push_back("SurfaceGui '" + surface.name + "': " + png_error);
            continue;
        }
        DrawItem quad;
        quad.name = surface.name + " (SurfaceGui)";
        quad.color = {1, 1, 1};
        quad.texture_path = tex_path;
        quad.cast_shadow = false;
        quad.receive_shadow = false;
        std::array<double, 3> centre{normal[0] * (face == 1 || face == 4 ? size[1] / 2 : face == 0 || face == 3 ? size[0] / 2 : size[2] / 2),
                                     normal[1] * (face == 1 || face == 4 ? size[1] / 2 : size[1] / 2),
                                     normal[2] * (face == 2 || face == 5 ? size[2] / 2 : size[2] / 2)};
        // Recompute centre properly per face: offset along normal by the
        // part's half-extent in that axis, nudged outward to avoid z-fighting.
        double axis_extent = (face == 0 || face == 3) ? size[0] / 2 : (face == 1 || face == 4) ? size[1] / 2 : size[2] / 2;
        centre = {normal[0] * (axis_extent + 0.02), normal[1] * (axis_extent + 0.02), normal[2] * (axis_extent + 0.02)};
        const double corners[4][2] = {{-1, -1}, {1, -1}, {1, 1}, {-1, 1}};
        const float uvs[4][2] = {{0, 1}, {1, 1}, {1, 0}, {0, 0}};
        uint32_t base = 0;
        for (int i = 0; i < 4; ++i) {
            std::array<double, 3> pos{
                centre[0] + u_axis[0] * corners[i][0] * hu + v_axis[0] * corners[i][1] * hv,
                centre[1] + u_axis[1] * corners[i][0] * hu + v_axis[1] * corners[i][1] * hv,
                centre[2] + u_axis[2] * corners[i][0] * hu + v_axis[2] * corners[i][1] * hv};
            quad.positions.push_back({static_cast<float>(pos[0]), static_cast<float>(pos[1]), static_cast<float>(pos[2])});
            quad.normals.push_back({static_cast<float>(normal[0]), static_cast<float>(normal[1]), static_cast<float>(normal[2])});
            quad.uvs.push_back({uvs[i][0], uvs[i][1]});
        }
        quad.indices = {base, base + 1, base + 2, base, base + 2, base + 3};
        quad.model = draw.model;
        scene.draws.push_back(std::move(quad));
    }
}

void walk(const Instance& inst, const SceneOptions& options, Scene& scene, const Instance& model_root_ref) {
    static const std::set<std::string> part_classes = {
        "Part", "MeshPart", "WedgePart", "CornerWedgePart", "SpawnLocation",
        "Seat", "TrussPart", "UnionOperation", "NegateOperation"};
    if (part_classes.count(inst.class_name)) extract_part(inst, options, scene);
    if (inst.class_name == "Beam" || inst.class_name == "Trail") {
        // Elongated quad strip between the two attachment world points.
        auto resolve = [&](const char* prop_name) -> const Instance* {
            auto it = inst.properties.find(prop_name);
            if (it == inst.properties.end() || it->second.type != ValueType::Ref) return nullptr;
            // Ref = child-index path; resolve against the model root.
            const Instance* cur = &model_root_ref;
            std::string path = it->second.string_value;
            size_t at = 0;
            while (at <= path.size() && cur) {
                size_t slash = path.find('/', at);
                std::string part = path.substr(at, slash == std::string::npos ? slash : slash - at);
                char* end = nullptr;
                long idx = std::strtol(part.c_str(), &end, 10);
                if (end == part.c_str() || idx < 0 || static_cast<size_t>(idx) >= cur->children.size()) return nullptr;
                cur = cur->children[static_cast<size_t>(idx)].get();
                if (slash == std::string::npos) break;
                at = slash + 1;
            }
            return cur;
        };
        const Instance* a0 = resolve("Attachment0");
        const Instance* a1 = resolve("Attachment1");
        if (a0 && a1 && a0->parent && a1->parent) {
            auto world_of = [](const Instance* attachment) {
                const Instance* part = attachment->parent;
                Variant pcf = Variant::make_cframe(0, 0, 0);
                if (auto it = part->properties.find("CFrame"); it != part->properties.end() && it->second.type == ValueType::CFrame) pcf = it->second;
                Variant acf = Variant::make_cframe(0, 0, 0);
                if (auto it = attachment->properties.find("CFrame"); it != attachment->properties.end() && it->second.type == ValueType::CFrame) acf = it->second;
                // world = part * local (rotation applied to offset)
                std::array<double, 3> pos{
                    pcf.position[0] + pcf.rotation[0] * acf.position[0] + pcf.rotation[1] * acf.position[1] + pcf.rotation[2] * acf.position[2],
                    pcf.position[1] + pcf.rotation[3] * acf.position[0] + pcf.rotation[4] * acf.position[1] + pcf.rotation[5] * acf.position[2],
                    pcf.position[2] + pcf.rotation[6] * acf.position[0] + pcf.rotation[7] * acf.position[1] + pcf.rotation[8] * acf.position[2]};
                return pos;
            };
            auto p0 = world_of(a0);
            auto p1 = world_of(a1);
            double dx = p1[0] - p0[0], dy = p1[1] - p0[1], dz = p1[2] - p0[2];
            double len = std::sqrt(dx * dx + dy * dy + dz * dz);
            if (len > 0.01) {
                DrawItem draw;
                draw.name = "Beam:" + inst.name;
                gen_box({0.2, 0.2, 1.0}, draw); // unit beam along Z, scaled below
                // Build a look-at style matrix: Z axis along the beam.
                std::array<double, 3> zax{dx / len, dy / len, dz / len};
                std::array<double, 3> up{0, 1, 0};
                if (std::abs(zax[1]) > 0.9) up = {1, 0, 0};
                std::array<double, 3> xax{up[1] * zax[2] - up[2] * zax[1],
                                          up[2] * zax[0] - up[0] * zax[2],
                                          up[0] * zax[1] - up[1] * zax[0]};
                double xl = std::sqrt(xax[0] * xax[0] + xax[1] * xax[1] + xax[2] * xax[2]);
                xax = {xax[0] / xl, xax[1] / xl, xax[2] / xl};
                std::array<double, 3> yax{zax[1] * xax[2] - zax[2] * xax[1],
                                          zax[2] * xax[0] - zax[0] * xax[2],
                                          zax[0] * xax[1] - zax[1] * xax[0]};
                double width = (prop_float(inst, "Width0", 1.0) + prop_float(inst, "Width1", 1.0)) / 2.0;
                draw.model = {
                    static_cast<float>(xax[0] * width), static_cast<float>(xax[1] * width), static_cast<float>(xax[2] * width), 0,
                    static_cast<float>(yax[0] * width), static_cast<float>(yax[1] * width), static_cast<float>(yax[2] * width), 0,
                    static_cast<float>(zax[0] * len), static_cast<float>(zax[1] * len), static_cast<float>(zax[2] * len), 0,
                    static_cast<float>((p0[0] + p1[0]) / 2), static_cast<float>((p0[1] + p1[1]) / 2),
                    static_cast<float>((p0[2] + p1[2]) / 2), 1};
                // Beam Colour: first ColorSequence keypoint when present.
                std::array<float, 3> col{1, 1, 1};
                if (auto it = inst.properties.find("Color"); it != inst.properties.end() &&
                    it->second.type == ValueType::ColorSequence && !it->second.complex_value.as_array().empty()) {
                    const Json& kp = it->second.complex_value.as_array()[0];
                    if (kp.find("color") && kp.find("color")->is_array() && kp.find("color")->as_array().size() == 3)
                        col = {static_cast<float>(kp.find("color")->as_array()[0].as_double()),
                               static_cast<float>(kp.find("color")->as_array()[1].as_double()),
                               static_cast<float>(kp.find("color")->as_array()[2].as_double())};
                }
                draw.color = srgb_to_linear(col);
                double beam_transparency = prop_float(inst, "Transparency", 0.0);
                if (auto it = inst.properties.find("Transparency");
                    it != inst.properties.end() && it->second.type == ValueType::NumberSequence &&
                    !it->second.complex_value.as_array().empty() &&
                    it->second.complex_value.as_array()[0].find("value")) {
                    beam_transparency = it->second.complex_value.as_array()[0].find("value")->as_double();
                }
                draw.transparency = static_cast<float>(beam_transparency);
                draw.material = 288; // Neon-ish emissive path
                draw.cast_shadow = false;
                scene.draws.push_back(std::move(draw));
            }
        }
    }
    if (inst.class_name == "ParticleEmitter" && inst.parent && DataModel::is_base_part(inst.parent->class_name)) {
        // Deterministic snapshot: seeds from the emitter path hash.
        const Instance* part = inst.parent;
        Variant pcf = Variant::make_cframe(0, 0, 0);
        if (auto it = part->properties.find("CFrame"); it != part->properties.end() && it->second.type == ValueType::CFrame) pcf = it->second;
        double rate = prop_float(inst, "Rate", 20.0);
        double lifetime = 2.0;
        if (auto it = inst.properties.find("Lifetime"); it != inst.properties.end() &&
            it->second.type == ValueType::NumberRange) {
            lifetime = (it->second.complex_value.as_array()[0].as_double() +
                        it->second.complex_value.as_array()[1].as_double()) / 2.0;
        }
        double speed = 5.0;
        if (auto it = inst.properties.find("Speed"); it != inst.properties.end() &&
            it->second.type == ValueType::NumberRange) {
            speed = (it->second.complex_value.as_array()[0].as_double() +
                     it->second.complex_value.as_array()[1].as_double()) / 2.0;
        }
        double size = 1.0;
        if (auto it = inst.properties.find("Size"); it != inst.properties.end() &&
            it->second.type == ValueType::NumberSequence && !it->second.complex_value.as_array().empty()) {
            const Json& kp = it->second.complex_value.as_array()[0];
            if (kp.find("value")) size = kp.find("value")->as_double();
        }
        double transparency = 0.0;
        if (auto it = inst.properties.find("Transparency"); it != inst.properties.end() &&
            it->second.type == ValueType::NumberSequence && !it->second.complex_value.as_array().empty()) {
            const Json& kp = it->second.complex_value.as_array()[0];
            if (kp.find("value")) transparency = kp.find("value")->as_double();
        }
        std::array<float, 3> col{1, 1, 1};
        if (auto it = inst.properties.find("Color"); it != inst.properties.end() &&
            it->second.type == ValueType::ColorSequence && !it->second.complex_value.as_array().empty()) {
            const Json& kp = it->second.complex_value.as_array()[0];
            if (kp.find("color") && kp.find("color")->is_array() && kp.find("color")->as_array().size() == 3)
                col = {static_cast<float>(kp.find("color")->as_array()[0].as_double()),
                       static_cast<float>(kp.find("color")->as_array()[1].as_double()),
                       static_cast<float>(kp.find("color")->as_array()[2].as_double())};
        }
        uint32_t seed = 2166136261u;
        for (char c : inst.name) { seed ^= static_cast<uint8_t>(c); seed *= 16777619u; }
        int count = static_cast<int>(std::min(200.0, rate * lifetime));
        for (int i = 0; i < count; ++i) {
            auto rnd = [&]() {
                seed = seed * 1664525u + 1013904223u;
                return (seed >> 8) / 16777216.0;
            };
            double age = rnd() * lifetime;
            double vx = (rnd() - 0.5) * 0.6, vy = 1.0, vz = (rnd() - 0.5) * 0.6;
            double vl = std::sqrt(vx * vx + vy * vy + vz * vz);
            std::array<double, 3> pos{
                pcf.position[0] + vx / vl * speed * age,
                pcf.position[1] + vy / vl * speed * age,
                pcf.position[2] + vz / vl * speed * age};
            DrawItem draw;
            draw.name = "Particle:" + inst.name;
            // Camera-facing quad (renderer expands it), unit sized.
            draw.positions = {{-0.5f, -0.5f, 0}, {0.5f, -0.5f, 0}, {0.5f, 0.5f, 0}, {-0.5f, 0.5f, 0}};
            draw.normals = {{0, 0, 1}, {0, 0, 1}, {0, 0, 1}, {0, 0, 1}};
            draw.uvs = {{0, 0}, {1, 0}, {1, 1}, {0, 1}};
            draw.indices = {0, 1, 2, 0, 2, 3};
            float s = static_cast<float>(size);
            draw.model = {s, 0, 0, 0, 0, s, 0, 0, 0, 0, 1, 0,
                          static_cast<float>(pos[0]), static_cast<float>(pos[1]), static_cast<float>(pos[2]), 1};
            draw.color = srgb_to_linear(col);
            draw.transparency = static_cast<float>(transparency);
            draw.material = 288;
            draw.billboard = true;
            draw.cast_shadow = false;
            draw.receive_shadow = false;
            scene.draws.push_back(std::move(draw));
        }
    }
    for (const auto& child : inst.children) walk(*child, options, scene, model_root_ref);
}

std::array<float, 3> color3_or(const Instance* inst, const char* name, std::array<float, 3> fallback) {
    if (!inst) return fallback;
    const Variant* v = prop(*inst, name);
    if (v && v->type == ValueType::Color3)
        return {static_cast<float>(v->vec3[0]), static_cast<float>(v->vec3[1]), static_cast<float>(v->vec3[2])};
    return fallback;
}

} // namespace

void collect_lights(const Instance& inst, const Instance* part, Scene& scene) {
    const Instance* current_part = part;
    if (DataModel::is_base_part(inst.class_name)) current_part = &inst;
    if ((inst.class_name == "PointLight" || inst.class_name == "SpotLight" ||
         inst.class_name == "SurfaceLight") && current_part) {
        if (scene.lighting.dynamic_lights.size() >= 8) {
            scene.warnings.push_back("more than 8 dynamic lights: '" + inst.name + "' dropped (shader cap)");
        } else {
            Variant cf = [&] {
                auto it = current_part->properties.find("CFrame");
                return (it != current_part->properties.end() && it->second.type == ValueType::CFrame)
                           ? it->second : Variant::make_cframe(0, 0, 0);
            }();
            DynamicLight light;
            light.position = {static_cast<float>(cf.position[0]), static_cast<float>(cf.position[1]),
                              static_cast<float>(cf.position[2])};
            auto col = color3_or(&inst, "Color", {1, 1, 1});
            double brightness = prop_float(inst, "Brightness", 1.0);
            std::array<float, 3> linear = srgb_to_linear(col);
            float scale = static_cast<float>(brightness * 2.0);
            light.color = {linear[0] * scale, linear[1] * scale, linear[2] * scale};
            light.range = static_cast<float>(prop_float(inst, "Range", 8.0));
            if (inst.class_name == "PointLight") {
                light.cone_cos = -2.0f;
            } else {
                // Direction: parent part's look vector (-Z column).
                light.direction = {static_cast<float>(-cf.rotation[2]), static_cast<float>(-cf.rotation[5]),
                                   static_cast<float>(-cf.rotation[8])};
                double angle = prop_float(inst, "Angle", 90.0);
                light.cone_cos = static_cast<float>(std::cos(angle * 0.5 * 3.141592653589793 / 180.0));
            }
            scene.lighting.dynamic_lights.push_back(light);
        }
    }
    for (const auto& child : inst.children) collect_lights(*child, current_part, scene);
}

Scene build_scene(const DataModel& model, const SceneOptions& options) {
    Scene scene;

    // ---- Lighting (Roblox Lighting service properties) ----
    const Instance* lighting = find_child_of_root(model, "Lighting");
    LightingInfo& li = scene.lighting;
    li.ambient = srgb_to_linear(color3_or(lighting, "Ambient", {0.2f, 0.2f, 0.2f}));
    li.outdoor_ambient = srgb_to_linear(color3_or(lighting, "OutdoorAmbient", {0.5f, 0.5f, 0.5f}));
    double brightness = lighting ? prop_float(*lighting, "Brightness", 2.0) : 2.0;
    double clock_time = lighting ? prop_float(*lighting, "ClockTime", 14.5) : 14.5;
    double latitude = lighting ? prop_float(*lighting, "GeographicLatitude", 41.733) : 41.733;
    li.shadows_enabled = lighting ? prop_bool(*lighting, "GlobalShadows", true) : true;

    // Sun path approximation: ClockTime 6 = sunrise, 12 = noon, 18 = sunset.
    // Roblox's exact solar curve depends on latitude/date; this is a
    // documented approximation (see COMPATIBILITY.md).
    const double pi = 3.14159265358979323846;
    double day_progress = (clock_time - 6.0) / 12.0; // 0..1 across the day
    double sun_angle = day_progress * pi;            // elevation curve
    double elevation = std::sin(sun_angle);
    double azimuth = day_progress * pi;              // east -> west
    double lat_tilt = std::cos(latitude * pi / 180.0) * 0.35;
    std::array<double, 3> sun_dir = {
        -std::cos(azimuth) * 0.8,
        -std::max(elevation, -0.25),
        0.45 + lat_tilt * 0.2,
    };
    double len = std::sqrt(sun_dir[0] * sun_dir[0] + sun_dir[1] * sun_dir[1] + sun_dir[2] * sun_dir[2]);
    li.sun_direction = {static_cast<float>(sun_dir[0] / len), static_cast<float>(sun_dir[1] / len),
                        static_cast<float>(sun_dir[2] / len)};
    float day_factor = static_cast<float>(std::clamp(elevation, 0.0, 1.0));
    float sun_scale = static_cast<float>(brightness) * (0.15f + 0.85f * day_factor);
    li.sun_color = {sun_scale * 1.0f, sun_scale * 0.95f, sun_scale * 0.86f};

    // Sky/background: gradient endpoints derived from time of day.
    li.has_sky = find_child_of_root(model, "Sky") != nullptr;
    auto mix3 = [](std::array<float, 3> a, std::array<float, 3> b, float t) {
        return std::array<float, 3>{a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t};
    };
    li.sky_top = mix3({0.05f, 0.07f, 0.16f}, {0.3f, 0.53f, 0.86f}, day_factor);
    li.sky_bottom = mix3({0.12f, 0.13f, 0.2f}, {0.74f, 0.82f, 0.92f}, day_factor);
    li.background = li.sky_bottom;

    // Fog (Roblox defaults: FogStart 0, FogEnd 100000 = effectively off).
    if (lighting) {
        li.fog_color = color3_or(lighting, "FogColor", li.background);
        li.fog_start = static_cast<float>(prop_float(*lighting, "FogStart", 0.0));
        double fog_end = prop_float(*lighting, "FogEnd", 100000.0);
        li.fog_end = static_cast<float>(fog_end);
    }

    // Atmosphere approximation: haze lightens the horizon colour.
    // Roblox parents Atmosphere under Lighting; also accept it at root.
    const Instance* atmo = nullptr;
    if (lighting) {
        for (const auto& child : lighting->children) {
            if (child->class_name == "Atmosphere") atmo = child.get();
        }
    }
    if (!atmo) atmo = find_child_of_root(model, "Atmosphere");
    if (atmo) {
        double density = prop_float(*atmo, "Density", 0.3);
        auto atmo_color = srgb_to_linear(color3_or(atmo, "Color", {0.78f, 0.83f, 0.92f}));
        float blend = static_cast<float>(std::clamp(density, 0.0, 1.0)) * 0.6f;
        li.sky_bottom = mix3(li.sky_bottom, atmo_color, blend);
        li.background = li.sky_bottom;
        if (li.fog_end > 9000.0f) { // no explicit fog: atmosphere implies haze distance
            li.fog_start = 120.0f;
            li.fog_end = static_cast<float>(900.0 - 600.0 * std::clamp(density, 0.0, 1.0));
            li.fog_color = li.sky_bottom;
        }
    }

    // ---- Current camera ----
    if (const Instance* workspace = find_child_of_root(model, "Workspace")) {
        collect_lights(*workspace, nullptr, scene);
    }
    // Terrain voxels -> merged box runs along X (Phase 11).
    if (const Instance* terrain = find_terrain(model)) {
        TerrainGrid grid;
        std::string terr_error;
        if (terrain_load(*terrain, grid, terr_error) && !grid.empty()) {
            // Group by (y, z, material), merge contiguous x runs.
            std::map<std::tuple<int, int, uint16_t>, std::vector<int>> rows;
            for (const auto& [key, value] : grid.cells) {
                auto [x, y, z] = key;
                rows[{y, z, value.first}].push_back(x);
            }
            for (auto& [row_key, xs] : rows) {
                auto [y, z, material] = row_key;
                std::sort(xs.begin(), xs.end());
                for (size_t i = 0; i < xs.size();) {
                    size_t j = i;
                    while (j + 1 < xs.size() && xs[j + 1] == xs[j] + 1) ++j;
                    double run = static_cast<double>(xs[j] - xs[i] + 1);
                    DrawItem draw;
                    draw.name = "Terrain";
                    gen_box({run * kVoxelSize, kVoxelSize, kVoxelSize}, draw);
                    auto first = voxel_center(xs[i], y, z);
                    auto last = voxel_center(xs[j], y, z);
                    draw.model = {1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0,
                                  static_cast<float>((first[0] + last[0]) / 2),
                                  static_cast<float>(first[1]),
                                  static_cast<float>(first[2]), 1};
                    draw.color = srgb_to_linear(terrain_material_color(material));
                    draw.material = material;
                    if (material == 2048) draw.transparency = 0.35f; // Water
                    draw.cast_shadow = false;
                    scene.draws.push_back(std::move(draw));
                    i = j + 1;
                }
            }
        } else if (!terr_error.empty()) {
            scene.warnings.push_back("Terrain: " + terr_error);
        }
    }
    if (const Instance* workspace = find_child_of_root(model, "Workspace")) {
        for (const auto& child : workspace->children) {
            if (child->class_name != "Camera") continue;
            scene.has_current_camera = true;
            scene.current_camera_name = child->name;
            CameraInfo& cam = scene.current_camera;
            const Variant* cf = prop(*child, "CFrame");
            if (cf && cf->type == ValueType::CFrame) {
                cam.position = {static_cast<float>(cf->position[0]), static_cast<float>(cf->position[1]),
                                static_cast<float>(cf->position[2])};
                // Camera looks along its -Z basis (third rotation column).
                const auto& r = cf->rotation;
                std::array<float, 3> forward = {-static_cast<float>(r[2]), -static_cast<float>(r[5]),
                                                -static_cast<float>(r[8])};
                cam.look_at = {cam.position[0] + forward[0], cam.position[1] + forward[1],
                               cam.position[2] + forward[2]};
                cam.up = {static_cast<float>(r[1]), static_cast<float>(r[4]), static_cast<float>(r[7])};
            }
            double fov = prop_float(*child, "FieldOfView", 70.0);
            cam.fov_degrees = static_cast<float>(fov);
            break;
        }
    }

    walk(model.root(), options, scene, model.root());
    for (const DrawItem& draw : scene.draws) {
        if (!draw.warning.empty()) scene.warnings.push_back(draw.name + ": " + draw.warning);
    }

    // Transparent draws render after opaque, sorted far-to-near by their
    // distance to the current camera position estimate (origin fallback).
    std::stable_partition(scene.draws.begin(), scene.draws.end(),
                          [](const DrawItem& d) { return d.transparency <= 0.0f; });
    return scene;
}

} // namespace blox
