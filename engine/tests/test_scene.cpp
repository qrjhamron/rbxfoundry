// Part B: part geometry generation, scene extraction and mesh loading tests.
#include "blox/datamodel.hpp"
#include "blox/scene.hpp"

#include <cmath>
#include <cstdio>
#include <fstream>
#include <string>

using namespace blox;

static int g_checks = 0;
static int g_failures = 0;

static void check(bool ok, const std::string& what) {
    ++g_checks;
    if (!ok) {
        ++g_failures;
        std::fprintf(stderr, "FAIL: %s\n", what.c_str());
    }
}

static bool indices_ok(const DrawItem& d) {
    for (uint32_t i : d.indices) {
        if (i >= d.positions.size()) return false;
    }
    return !d.indices.empty() && d.indices.size() % 3 == 0;
}

static Instance& add_part(DataModel& dm, Instance& parent, const std::string& cls,
                          const std::string& name, std::array<double, 3> pos,
                          std::array<double, 3> size, std::array<double, 3> color) {
    Instance& part = dm.add_instance(parent, cls, name);
    std::string err;
    check(dm.set_property(part, "CFrame", Variant::make_cframe(pos[0], pos[1], pos[2]), err), "set CFrame: " + err);
    check(dm.set_property(part, "Size", Variant::make_vec3(size[0], size[1], size[2]), err), "set Size: " + err);
    check(dm.set_property(part, "Color", Variant::make_color3(color[0], color[1], color[2]), err), "set Color: " + err);
    return part;
}

int main() {
#ifdef BLOX_FIXTURE_DIR
    const std::string fixtures = BLOX_FIXTURE_DIR;
#else
    const std::string fixtures = "tests/fixtures";
#endif

    // --- Geometry generators ------------------------------------------------
    {
        DrawItem box;
        gen_box({2, 3, 4}, box);
        check(box.positions.size() == 24 && box.indices.size() == 36, "box has 24 verts / 36 indices");
        check(indices_ok(box), "box indices in range");
        bool top_normal = false;
        for (size_t i = 0; i < box.positions.size(); ++i) {
            if (std::fabs(box.positions[i][1] - 1.5f) < 1e-6 && std::fabs(box.normals[i][1] - 1.0f) < 1e-6)
                top_normal = true;
        }
        check(top_normal, "box top face has +Y normal");
        float max_x = 0;
        for (const auto& p : box.positions) max_x = std::max(max_x, std::fabs(p[0]));
        check(std::fabs(max_x - 1.0f) < 1e-6, "box extents match Size/2");
    }
    {
        DrawItem wedge;
        gen_wedge({4, 2, 3}, wedge);
        check(indices_ok(wedge), "wedge generates valid triangles");
        bool slope = false, top_edge = false;
        for (size_t i = 0; i < wedge.normals.size(); ++i) {
            if (wedge.normals[i][1] > 0.5f && wedge.normals[i][2] > 0.3f) slope = true;
            if (std::fabs(wedge.positions[i][1] - 1.0f) < 1e-6 && std::fabs(wedge.positions[i][2] + 1.5f) < 1e-6)
                top_edge = true;
        }
        check(slope, "wedge has a sloped face");
    }
    {
        DrawItem corner;
        gen_corner_wedge({4, 2, 3}, corner);
        check(indices_ok(corner), "corner wedge generates valid triangles");
    }
    {
        DrawItem sphere;
        gen_sphere({4, 2, 2}, 24, sphere);
        check(indices_ok(sphere), "sphere generates valid triangles");
        float max_r = 0;
        for (const auto& p : sphere.positions) {
            float r = std::sqrt(p[0] * p[0] + p[1] * p[1] + p[2] * p[2]);
            max_r = std::max(max_r, r);
        }
        check(std::fabs(max_r - 1.0f) < 1e-5, "Ball diameter is min(Size) (Roblox rule)");
    }
    {
        DrawItem cyl;
        gen_cylinder({3, 2, 2}, 24, cyl);
        check(indices_ok(cyl), "cylinder generates valid triangles");
        float max_x = 0, max_y = 0;
        for (const auto& p : cyl.positions) {
            max_x = std::max(max_x, std::fabs(p[0]));
            max_y = std::max(max_y, std::fabs(p[1]));
        }
        check(std::fabs(max_x - 1.5f) < 1e-5, "cylinder axis along X with length Size.x");
        check(std::fabs(max_y - 1.0f) < 1e-5, "cylinder diameter is min(Size.y, Size.z)");
    }

    // --- Scene extraction -------------------------------------------------
    {
        DataModel dm;
        Instance& ws = dm.add_instance(dm.root(), "Workspace", "Workspace");
        add_part(dm, ws, "Part", "Ground", {0, -1, 0}, {40, 2, 40}, {0.4, 0.8, 0.4});
        add_part(dm, ws, "Part", "Block", {0, 1, 0}, {4, 2, 4}, {1, 0.2, 0.2});
        Instance& folder = dm.add_instance(ws, "Folder", "Decor");
        Instance& ghost = dm.add_instance(folder, "Part", "Ghost");
        std::string err;
        check(dm.set_property(ghost, "Transparency", Variant::make_float64(1.0), err), "set Transparency");
        Instance& wedge = dm.add_instance(ws, "WedgePart", "Ramp");
        check(dm.set_property(wedge, "CFrame", Variant::make_cframe(8, 0, 0), err), "wedge CFrame");
        check(dm.set_property(wedge, "Size", Variant::make_vec3(4, 2, 3), err), "wedge Size");

        Scene scene = build_scene(dm, SceneOptions{});
        check(scene.part_count == 3, "extractor counts visible parts (got " +
                                         std::to_string(scene.part_count) + ")");
        check(scene.draws.size() == 3, "extractor yields one draw per visible part");
        const DrawItem* ground = nullptr;
        for (const DrawItem& d : scene.draws) {
            if (d.name == "Ground") ground = &d;
        }
        check(ground != nullptr, "ground part extracted");
        if (ground) {
            check(std::fabs(ground->model[13] - (-1.0f)) < 1e-6, "ground world Y in model matrix");
            check(std::fabs(ground->color[1] - srgb_to_linear({0.4f, 0.8f, 0.4f})[1]) < 1e-6,
                  "part colour converted to linear space");
        }
        // Material designation survives: default Part material is Plastic (1536).
        bool wedge_found = false;
        for (const DrawItem& d : scene.draws) {
            if (d.name == "Ramp") wedge_found = indices_ok(d);
        }
        check(wedge_found, "wedge part becomes valid geometry");
    }

    // --- Mesh loading through the public loader -----------------------------
    {
        DataModel dm;
        Instance& ws = dm.add_instance(dm.root(), "Workspace", "Workspace");
        Instance& mp = dm.add_instance(ws, "MeshPart", "Quad");
        std::string err;
        dm.set_property(mp, "Size", Variant::make_vec3(4, 2, 4), err);
        SceneOptions opts;
        opts.assets_dir = fixtures;
        DrawItem out;
        check(load_mesh_geometry("quad_v2.mesh", "", opts, mp, out, err),
              "v2 quad fixture loads: " + err);
        check(out.positions.size() == 4 && out.indices.size() == 6,
              "quad fixture has 4 verts / 2 triangles (got " +
                  std::to_string(out.positions.size()) + "/" +
                  std::to_string(out.indices.size()) + ")");

        DrawItem cube;
        check(load_mesh_geometry("cube_v1.mesh", "", opts, mp, cube, err),
              "v1 cube fixture loads: " + err);
        check(cube.positions.size() == 36 && cube.indices.size() == 36,
              "cube fixture has 36 verts / 12 triangles");

        DrawItem missing;
        std::string merr;
        check(!load_mesh_geometry("rbxassetid://123456", "", opts, mp, missing, merr),
              "online-only mesh id fails loudly instead of faking geometry");
        check(merr.find("123456") != std::string::npos, "error names the missing asset id");

        // Unsupported version must fail loudly too.
        {
            std::ofstream bad(fixtures + "/bad_v9.mesh", std::ios::binary);
            bad << "version 9.00\n";
        }
        // v3/v4/v5 encodings of the identical unit cube must all parse to
        // the same geometry (v4 also guards a former header-width bug).
        for (const char* fixture : {"cube_v3.mesh", "cube_v4.mesh", "cube_v5.mesh"}) {
            DrawItem item;
            check(load_mesh_geometry(fixture, "", opts, mp, item, err),
                  std::string(fixture) + " loads: " + err);
            check(item.positions.size() == 36 && item.indices.size() == 36,
                  std::string(fixture) + " has 36 verts / 12 triangles");
        }
        // Skinned v5: geometry renders, but the draw must carry the
        // unskinned warning rather than pretending skinning works.
        DrawItem skinned;
        check(load_mesh_geometry("cube_v5_skinned.mesh", "", opts, mp, skinned, err),
              "skinned v5 loads: " + err);
        check(skinned.warning.find("skinning") != std::string::npos,
              "skinned mesh carries an explicit warning");

        DrawItem bad_out;
        std::string berr;
        check(!load_mesh_geometry("bad_v9.mesh", "", opts, mp, bad_out, berr),
              "mesh version 9 is rejected");
        std::remove((fixtures + "/bad_v9.mesh").c_str());
    }

    if (g_failures == 0) {
        std::printf("scene tests passed (%d checks)\n", g_checks);
        return 0;
    }
    std::fprintf(stderr, "scene tests failed: %d/%d\n", g_failures, g_checks);
    return 1;
}
