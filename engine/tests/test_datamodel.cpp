// Focused DataModel tests: tree mutation, path resolution, type checks,
// snapshot round-trip through JSON. No test framework; plain asserts with
// process exit code as the result (run via CTest).
#include <cassert>
#include <cmath>
#include <iostream>

#include "blox/datamodel/datamodel.hpp"

using namespace blox;

static int failures = 0;
#define CHECK(cond) do { if (!(cond)) { std::cerr << "FAIL line " << __LINE__ << ": " #cond "\n"; ++failures; } } while (0)

int main() {
    DataModel model;
    model.reset("TestPlace");
    CHECK(model.root().class_name == "DataModel");
    CHECK(model.instance_count() == 1);

    std::string err;
    Instance& workspace = model.add_instance(model.root(), "Workspace", "Workspace");
    Instance& part = model.add_instance(workspace, "Part", "Baseplate");
    CHECK(model.instance_count() == 3);

    // Reflection defaults were applied.
    auto size_it = part.properties.find("Size");
    CHECK(size_it != part.properties.end());
    CHECK(size_it->second.type == ValueType::Vector3);
    CHECK(size_it->second.vec3[0] == 4 && size_it->second.vec3[1] == 1 && size_it->second.vec3[2] == 2);
    CHECK(part.properties.at("Anchored").bool_value == false);

    // Path resolution, with and without the game prefix.
    CHECK(model.resolve_path("Workspace/Baseplate", err) == &part);
    CHECK(model.resolve_path("game/Workspace/Baseplate", err) == &part);
    CHECK(model.resolve_path("Workspace/Missing", err) == nullptr);
    CHECK(!err.empty());

    // Typed set + type mismatch rejection.
    CHECK(model.set_property(part, "Transparency", Variant::make_float64(0.5), err));
    CHECK(part.properties.at("Transparency").float_value == 0.5);
    CHECK(!model.set_property(part, "Transparency", Variant::make_string("x"), err));
    CHECK(!err.empty());

    // Rename through the Name pseudo-property.
    CHECK(model.set_property(part, "Name", Variant::make_string("Floor"), err));
    CHECK(model.resolve_path("Workspace/Floor", err) == &part);

    // Duplicate sibling names make paths ambiguous: an error, not first match.
    model.add_instance(workspace, "Part", "Dup");
    model.add_instance(workspace, "Part", "Dup");
    CHECK(model.resolve_path("Workspace/Dup", err) == nullptr);
    CHECK(err.find("ambiguous") != std::string::npos);

    // Snapshot -> JSON text -> parse -> load must reproduce the tree.
    std::string text = model.snapshot().dump();
    Json parsed;
    std::string parse_err;
    CHECK(Json::parse(text, parsed, parse_err));
    DataModel restored;
    CHECK(restored.load_snapshot(parsed, err));
    CHECK(restored.instance_count() == model.instance_count());
    Instance* floor = restored.resolve_path("Workspace/Floor", err);
    CHECK(floor != nullptr);
    if (floor) {
        CHECK(floor->class_name == "Part");
        CHECK(floor->properties.at("Transparency").float_value == 0.5);
    }

    // Unknown classes/properties survive a round-trip (no silent data loss).
    Instance& custom = model.add_instance(workspace, "MyCustomClass", "Custom");
    Variant weird;
    weird.type = ValueType::Vector3;
    weird.vec3 = {1, 2, 3};
    CHECK(model.set_property(custom, "WeirdProp", weird, err));
    auto issues = model.validate();
    bool saw_unknown_class = false;
    for (const auto& issue : issues)
        if (issue.message.find("unknown class 'MyCustomClass'") != std::string::npos) saw_unknown_class = true;
    CHECK(saw_unknown_class);

    // A mistyped known property is a validation error.
    Instance& bad = model.add_instance(workspace, "Part", "Bad");
    bad.properties["Transparency"] = Variant::make_string("not-a-number");
    bool saw_type_error = false;
    for (const auto& issue : model.validate())
        if (issue.severity == "error" && issue.message.find("Transparency") != std::string::npos) saw_type_error = true;
    CHECK(saw_type_error);

    // ---- Phase 2: hierarchy ops ----
    {
        DataModel m;
        m.reset("P2");
        std::string e;
        Instance& ws = m.add_instance(m.root(), "Workspace", "Workspace");
        Instance& model = m.add_instance(ws, "Model", "Rig");
        Instance& a = m.add_instance(model, "Part", "A");
        Instance& b = m.add_instance(ws, "Part", "B");
        CHECK(m.set_property(a, "CFrame", Variant::make_cframe(10, 0, 0), e));
        CHECK(m.set_property(b, "CFrame", Variant::make_cframe(-10, 0, 0), e));

        // Clone deep-copies with fresh referents.
        Instance& copy = m.clone_instance(model, ws, "RigCopy");
        CHECK(copy.referent != model.referent);
        CHECK(copy.children.size() == 1);
        CHECK(copy.children[0]->referent != a.referent);
        CHECK(copy.children[0]->properties.at("CFrame").position[0] == 10);

        // Reparent + cycle rejection.
        CHECK(m.reparent(b, model, e));
        CHECK(m.resolve_path("Workspace/Rig/B", e) == &b);
        CHECK(!m.reparent(model, b, e)); // b is inside model: cycle
        CHECK(!e.empty());

        // Find by class and by substring.
        auto parts = m.find({"", "Part", false}, nullptr);
        CHECK(parts.size() == 3);
        auto rigs = m.find({"Rig", "", true}, nullptr);
        CHECK(rigs.size() == 2); // Rig and RigCopy

        // Group moves targets under a new Model without changing world CFrames.
        Instance& loose = m.add_instance(ws, "Part", "Loose");
        CHECK(m.set_property(loose, "CFrame", Variant::make_cframe(1, 2, 3), e));
        Instance& group = m.group_instances({&loose}, ws, "Grouped");
        CHECK(group.class_name == "Model");
        CHECK(m.resolve_path("Workspace/Grouped/Loose", e) == &loose);
        CHECK(loose.properties.at("CFrame").position[1] == 2);

        // Delete removes the subtree.
        size_t before = m.instance_count();
        size_t removed = m.delete_instance(copy);
        CHECK(removed == 2);
        CHECK(m.instance_count() == before - 2);
        CHECK(m.resolve_path("Workspace/RigCopy", e) == nullptr);
    }

    // ---- Phase 2: transforms ----
    {
        DataModel m;
        m.reset("P2T");
        std::string e;
        auto near = [](double x, double y) { return std::abs(x - y) < 1e-9; };
        Instance& ws = m.add_instance(m.root(), "Workspace", "Workspace");
        Instance& model = m.add_instance(ws, "Model", "M");
        Instance& p1 = m.add_instance(model, "Part", "P1");
        Instance& p2 = m.add_instance(model, "Part", "P2");
        CHECK(m.set_property(p1, "CFrame", Variant::make_cframe(2, 0, 0), e));
        CHECK(m.set_property(p2, "CFrame", Variant::make_cframe(-2, 0, 0), e));
        CHECK(m.set_property(p1, "Size", Variant::make_vec3(2, 2, 2), e));
        CHECK(m.set_property(p2, "Size", Variant::make_vec3(2, 2, 2), e));

        // Model pivot defaults to the bounding-box centre (here: origin).
        Variant pivot = m.model_pivot(model);
        CHECK(near(pivot.position[0], 0) && near(pivot.position[1], 0));

        // Moving a Model translates every descendant part.
        CHECK(m.move(model, {0, 5, 0}, false, e));
        CHECK(near(p1.properties.at("CFrame").position[1], 5));
        CHECK(near(p2.properties.at("CFrame").position[1], 5));

        // Rotating a Model by 90° about Y around its pivot maps +X to -Z.
        CHECK(m.rotate(model, {0, 90, 0}, false, e));
        const auto& cf1 = p1.properties.at("CFrame");
        CHECK(near(cf1.position[0], 0) && near(cf1.position[2], -2));
        CHECK(near(cf1.rotation[0], 0) && near(cf1.rotation[6], -1)); // X axis (column 0) now -Z

        // Scaling a Model uniformly doubles sizes and pivot-relative offsets.
        CHECK(m.scale(model, {2, 2, 2}, e));
        CHECK(near(p1.properties.at("Size").vec3[0], 4));
        CHECK(near(p1.properties.at("CFrame").position[2], -4));
        CHECK(!m.scale(model, {2, 1, 2}, e)); // non-uniform model scale rejected

        // Part rotate is about its own centre: position unchanged.
        auto pos_before = p2.properties.at("CFrame").position;
        CHECK(m.rotate(p2, {0, 0, 90}, false, e));
        CHECK(p2.properties.at("CFrame").position == pos_before);

        // Relative move follows the part's local axes. p2 has been rotated
        // 90° about Y (model) and 90° about Z (local); just verify it moved.
        auto before2 = p2.properties.at("CFrame").position;
        CHECK(m.move(p2, {1, 0, 0}, true, e));
        auto after2 = p2.properties.at("CFrame").position;
        double dist = std::sqrt((after2[0]-before2[0])*(after2[0]-before2[0]) +
                                (after2[1]-before2[1])*(after2[1]-before2[1]) +
                                (after2[2]-before2[2])*(after2[2]-before2[2]));
        CHECK(near(dist, 1.0));

        // Explicit pivot is honoured for rotation.
        CHECK(m.set_model_pivot(model, Variant::make_cframe(100, 0, 0), e));
        auto pb = p1.properties.at("CFrame").position;
        CHECK(m.rotate(model, {0, 90, 0}, false, e));
        auto pa = p1.properties.at("CFrame").position;
        // Rotating about x=100 must actually displace the part.
        CHECK(std::abs(pa[0] - pb[0]) > 1.0 || std::abs(pa[2] - pb[2]) > 1.0);
        CHECK(!m.set_model_pivot(p1, Variant::make_cframe(0, 0, 0), e)); // not a Model

        // Folder cannot be moved.
        Instance& folder = m.add_instance(ws, "Folder", "F");
        CHECK(!m.move(folder, {1, 0, 0}, false, e));
    }

    if (failures == 0) std::cout << "test_datamodel: all checks passed\n";
    return failures == 0 ? 0 : 1;
}
