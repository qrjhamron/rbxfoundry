// Hierarchy transforms: world-space CFrame math for move/rotate/scale,
// Model pivots, clone/delete/reparent/group and find. Separated from the
// core tree/property logic in datamodel.cpp; shared helpers come from
// datamodel_internal.hpp.
#include "blox/datamodel/datamodel.hpp"
#include "datamodel_internal.hpp"

#include <algorithm>
#include <cmath>
#include <functional>
#include <set>

namespace blox {

// ================= Phase 2: hierarchy editing & transforms =================
namespace {

using Vec3 = std::array<double, 3>;
using Mat3 = std::array<double, 9>; // row-major; basis axes are the columns

Mat3 mat_mul(const Mat3& a, const Mat3& b) {
    Mat3 c{};
    for (int r = 0; r < 3; ++r)
        for (int col = 0; col < 3; ++col) {
            double s = 0;
            for (int k = 0; k < 3; ++k) s += a[r * 3 + k] * b[k * 3 + col];
            c[r * 3 + col] = s;
        }
    return c;
}
Vec3 mat_vec(const Mat3& m, const Vec3& v) {
    return {m[0] * v[0] + m[1] * v[1] + m[2] * v[2],
            m[3] * v[0] + m[4] * v[1] + m[5] * v[2],
            m[6] * v[0] + m[7] * v[1] + m[8] * v[2]};
}
Vec3 vec_add(const Vec3& a, const Vec3& b) { return {a[0] + b[0], a[1] + b[1], a[2] + b[2]}; }
Vec3 vec_sub(const Vec3& a, const Vec3& b) { return {a[0] - b[0], a[1] - b[1], a[2] - b[2]}; }
Vec3 vec_scale(const Vec3& a, double s) { return {a[0] * s, a[1] * s, a[2] * s}; }

Mat3 euler_xyz_deg(const Vec3& deg) {
    constexpr double kPi = 3.14159265358979323846;
    auto rad = [&](double d) { return d * kPi / 180.0; };
    double cx = std::cos(rad(deg[0])), sx = std::sin(rad(deg[0]));
    double cy = std::cos(rad(deg[1])), sy = std::sin(rad(deg[1]));
    double cz = std::cos(rad(deg[2])), sz = std::sin(rad(deg[2]));
    Mat3 rx = {1, 0, 0, 0, cx, -sx, 0, sx, cx};
    Mat3 ry = {cy, 0, sy, 0, 1, 0, -sy, 0, cy};
    Mat3 rz = {cz, -sz, 0, sz, cz, 0, 0, 0, 1};
    // Intrinsic XYZ: rotate about X, then the new Y, then the new Z.
    return mat_mul(rx, mat_mul(ry, rz));
}

Variant get_cframe(const Instance& inst) {
    auto it = inst.properties.find("CFrame");
    if (it != inst.properties.end() && it->second.type == ValueType::CFrame) return it->second;
    return Variant::make_cframe(0, 0, 0);
}
void set_cframe(Instance& inst, const Variant& cf) {
    inst.properties["CFrame"] = cf;
    detail::sync_position_from_cframe(inst);
}
Variant get_size(const Instance& inst) {
    auto it = inst.properties.find("Size");
    if (it != inst.properties.end() && it->second.type == ValueType::Vector3) return it->second;
    return Variant::make_vec3(0, 0, 0);
}
bool is_identity_cframe(const Variant& cf) {
    static const std::array<double, 9> ident = {1, 0, 0, 0, 1, 0, 0, 0, 1};
    return cf.position == Vec3{0, 0, 0} && cf.rotation == ident;
}
size_t subtree_count(const Instance& inst) {
    size_t n = 1;
    for (const auto& child : inst.children) n += subtree_count(*child);
    return n;
}
bool is_ancestor_of(const Instance* maybe_ancestor, const Instance* inst) {
    for (const Instance* cur = inst; cur; cur = cur->parent)
        if (cur == maybe_ancestor) return true;
    return false;
}

} // namespace

bool DataModel::is_base_part(const std::string& class_name) {
    static const std::set<std::string> parts = {
        "Part", "MeshPart", "SpawnLocation", "Seat", "WedgePart", "CornerWedgePart",
        "TrussPart", "UnionOperation", "NegateOperation"};
    return parts.count(class_name) > 0;
}

std::unique_ptr<Instance> DataModel::clone_rec(const Instance& source, Instance* new_parent) {
    auto copy = std::make_unique<Instance>();
    copy->referent = alloc_referent();
    copy->class_name = source.class_name;
    copy->name = source.name;
    copy->properties = source.properties;
    copy->parent = new_parent;
    for (const auto& child : source.children) copy->children.push_back(clone_rec(*child, copy.get()));
    return copy;
}

Instance& DataModel::clone_instance(const Instance& source, Instance& new_parent, const std::string& name_override) {
    auto copy = clone_rec(source, &new_parent);
    if (!name_override.empty()) copy->name = name_override;
    Instance& ref = *copy;
    new_parent.children.push_back(std::move(copy));
    return ref;
}

size_t DataModel::delete_instance(Instance& inst) {
    size_t count = subtree_count(inst);
    if (Instance* parent = inst.parent) {
        auto& siblings = parent->children;
        siblings.erase(std::remove_if(siblings.begin(), siblings.end(),
                                       [&](const std::unique_ptr<Instance>& c) { return c.get() == &inst; }),
                       siblings.end());
    }
    return count;
}

bool DataModel::reparent(Instance& inst, Instance& new_parent, std::string& error) {
    if (&inst == root_.get()) { error = "the DataModel root cannot be reparented"; return false; }
    if (is_ancestor_of(&inst, &new_parent)) { error = "cannot parent an instance to itself or its own descendant (cycle)"; return false; }
    Instance* old_parent = inst.parent;
    if (!old_parent) { error = "instance has no parent"; return false; }
    if (old_parent == &new_parent) return true;
    auto& siblings = old_parent->children;
    auto it = std::find_if(siblings.begin(), siblings.end(),
                           [&](const std::unique_ptr<Instance>& c) { return c.get() == &inst; });
    if (it == siblings.end()) { error = "instance not found under its parent"; return false; }
    std::unique_ptr<Instance> owned = std::move(*it);
    siblings.erase(it);
    owned->parent = &new_parent;
    new_parent.children.push_back(std::move(owned));
    return true;
}

Instance& DataModel::group_instances(std::vector<Instance*> targets, Instance& parent, const std::string& name) {
    Instance& model = add_instance(parent, "Model", name.empty() ? "Model" : name);
    std::string ignored;
    for (Instance* target : targets) {
        // Targets are moved as-is: BasePart CFrames are world-space in
        // Roblox, so grouping never changes a part's world transform.
        reparent(*target, model, ignored);
    }
    return model;
}

void DataModel::find_rec(const FindCriteria& criteria, Instance& inst, std::vector<Instance*>& out) const {
    bool name_ok = criteria.name.empty() ||
                   (criteria.substring ? inst.name.find(criteria.name) != std::string::npos
                                       : inst.name == criteria.name);
    bool class_ok = criteria.class_name.empty() || inst.class_name == criteria.class_name;
    if (name_ok && class_ok) out.push_back(&inst);
    for (const auto& child : inst.children) find_rec(criteria, *child, out);
}

std::vector<Instance*> DataModel::find(const FindCriteria& criteria, Instance* within) const {
    std::vector<Instance*> out;
    find_rec(criteria, within ? *within : *root_, out);
    return out;
}

void DataModel::collect_parts(Instance& inst, std::vector<Instance*>& out) {
    if (is_base_part(inst.class_name)) out.push_back(&inst);
    for (const auto& child : inst.children) collect_parts(*child, out);
}
void DataModel::collect_models(Instance& inst, std::vector<Instance*>& out) {
    if (inst.class_name == "Model") out.push_back(&inst);
    for (const auto& child : inst.children) collect_models(*child, out);
}

Variant DataModel::model_pivot(const Instance& model) const {
    auto it = model.properties.find("WorldPivot");
    if (it != model.properties.end() && it->second.type == ValueType::CFrame && !is_identity_cframe(it->second))
        return it->second;
    // Bounding-box centre of descendant BaseParts (each part's rotated box).
    std::vector<const Instance*> parts;
    std::function<void(const Instance&)> walk = [&](const Instance& inst) {
        if (is_base_part(inst.class_name)) parts.push_back(&inst);
        for (const auto& child : inst.children) walk(*child);
    };
    walk(model);
    if (parts.empty()) return it != model.properties.end() ? it->second : Variant::make_cframe(0, 0, 0);
    Vec3 lo{1e30, 1e30, 1e30}, hi{-1e30, -1e30, -1e30};
    for (const Instance* part : parts) {
        Variant cf = get_cframe(*part);
        Variant size = get_size(*part);
        for (int mask = 0; mask < 8; ++mask) {
            Vec3 local{(mask & 1 ? 0.5 : -0.5) * size.vec3[0],
                       (mask & 2 ? 0.5 : -0.5) * size.vec3[1],
                       (mask & 4 ? 0.5 : -0.5) * size.vec3[2]};
            Vec3 world = vec_add(cf.position, mat_vec(cf.rotation, local));
            for (int i = 0; i < 3; ++i) { lo[i] = std::min(lo[i], world[i]); hi[i] = std::max(hi[i], world[i]); }
        }
    }
    Vec3 centre = vec_scale(vec_add(lo, hi), 0.5);
    return Variant::make_cframe(centre[0], centre[1], centre[2]);
}

bool DataModel::set_model_pivot(Instance& model, const Variant& cframe, std::string& error) {
    if (model.class_name != "Model") { error = "pivot can only be set on a Model, got " + model.class_name; return false; }
    if (cframe.type != ValueType::CFrame) { error = "pivot must be a CFrame"; return false; }
    model.properties["WorldPivot"] = cframe;
    return true;
}

bool DataModel::move(Instance& inst, const std::array<double, 3>& delta, bool relative, std::string& error) {
    if (is_base_part(inst.class_name)) {
        Variant cf = get_cframe(inst);
        Vec3 d = relative ? mat_vec(cf.rotation, delta) : delta;
        cf.position = vec_add(cf.position, d);
        set_cframe(inst, cf);
        // Keep a stored Position property in sync via set_cframe.
        return true;
    }
    if (inst.class_name == "Model") {
        std::vector<Instance*> parts;
        collect_parts(inst, parts);
        std::vector<Instance*> models;
        collect_models(inst, models);
        for (Instance* part : parts) {
            Variant cf = get_cframe(*part);
            cf.position = vec_add(cf.position, delta);
            set_cframe(*part, cf);
        }
        for (Instance* sub : models) {
            auto it = sub->properties.find("WorldPivot");
            if (it != sub->properties.end() && it->second.type == ValueType::CFrame) {
                it->second.position = vec_add(it->second.position, delta);
            }
        }
        return true;
    }
    error = "'" + inst.name + "' (" + inst.class_name + ") has no world transform to move; move a BasePart or a Model";
    return false;
}

bool DataModel::rotate(Instance& inst, const std::array<double, 3>& angles_deg, bool world_space, std::string& error) {
    Mat3 r = euler_xyz_deg(angles_deg);
    if (is_base_part(inst.class_name)) {
        Variant cf = get_cframe(inst);
        cf.rotation = world_space ? mat_mul(r, cf.rotation) : mat_mul(cf.rotation, r);
        set_cframe(inst, cf);
        return true;
    }
    if (inst.class_name == "Model") {
        Variant pivot = model_pivot(inst);
        Vec3 p = pivot.position;
        std::vector<Instance*> parts;
        collect_parts(inst, parts);
        for (Instance* part : parts) {
            Variant cf = get_cframe(*part);
            cf.position = vec_add(p, mat_vec(r, vec_sub(cf.position, p)));
            cf.rotation = mat_mul(r, cf.rotation);
            set_cframe(*part, cf);
        }
        std::vector<Instance*> models;
        collect_models(inst, models);
        for (Instance* sub : models) {
            auto it = sub->properties.find("WorldPivot");
            if (it != sub->properties.end() && it->second.type == ValueType::CFrame) {
                it->second.position = vec_add(p, mat_vec(r, vec_sub(it->second.position, p)));
                it->second.rotation = mat_mul(r, it->second.rotation);
            }
        }
        // The model's own pivot orientation rotates too (position is the centre).
        Variant own = model_pivot(inst);
        own.rotation = mat_mul(r, own.rotation);
        auto it = inst.properties.find("WorldPivot");
        if (it != inst.properties.end() && it->second.type == ValueType::CFrame) it->second = own;
        return true;
    }
    error = "'" + inst.name + "' (" + inst.class_name + ") has no world transform to rotate; rotate a BasePart or a Model";
    return false;
}

bool DataModel::scale(Instance& inst, const std::array<double, 3>& factors, std::string& error) {
    for (double f : factors)
        if (!(f > 0.0)) { error = "scale factors must be positive numbers"; return false; }
    if (is_base_part(inst.class_name)) {
        Variant size = get_size(inst);
        for (int i = 0; i < 3; ++i) size.vec3[i] *= factors[i];
        inst.properties["Size"] = size;
        return true;
    }
    if (inst.class_name == "Model") {
        if (factors[0] != factors[1] || factors[1] != factors[2]) {
            error = "Models can only be scaled uniformly (Roblox Model:ScaleTo semantics); use a single factor";
            return false;
        }
        double f = factors[0];
        Variant pivot = model_pivot(inst);
        Vec3 p = pivot.position;
        std::vector<Instance*> parts;
        collect_parts(inst, parts);
        for (Instance* part : parts) {
            Variant cf = get_cframe(*part);
            cf.position = vec_add(p, vec_scale(vec_sub(cf.position, p), f));
            set_cframe(*part, cf);
            Variant size = get_size(*part);
            for (int i = 0; i < 3; ++i) size.vec3[i] *= f;
            part->properties["Size"] = size;
        }
        std::vector<Instance*> models;
        collect_models(inst, models);
        for (Instance* sub : models) {
            if (sub == &inst) continue;
            auto it = sub->properties.find("WorldPivot");
            if (it != sub->properties.end() && it->second.type == ValueType::CFrame)
                it->second.position = vec_add(p, vec_scale(vec_sub(it->second.position, p), f));
        }
        return true;
    }
    error = "'" + inst.name + "' (" + inst.class_name + ") cannot be scaled; scale a BasePart or a Model";
    return false;
}

} // namespace blox
