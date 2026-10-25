// DataModel core: instance tree ownership, path resolution, property
// semantics (including Color<->BrickColor sync), snapshot load/save and
// validation. Reflection defaults live in reflection.cpp; hierarchy
// transforms live in transforms.cpp.
#include "blox/datamodel/datamodel.hpp"
#include "blox/core/brickcolor.hpp"
#include "datamodel_internal.hpp"

#include <algorithm>
#include <cmath>
#include <functional>
#include <set>

namespace blox {

namespace detail {
void sync_position_from_cframe(Instance& inst) {
    auto pos = inst.properties.find("Position");
    auto cf = inst.properties.find("CFrame");
    if (pos != inst.properties.end() && cf != inst.properties.end() &&
        pos->second.type == ValueType::Vector3 && cf->second.type == ValueType::CFrame) {
        pos->second.vec3 = cf->second.position;
    }
}

std::vector<std::string> split_path(const std::string& path) {
    std::vector<std::string> parts;
    std::string cur;
    for (char c : path) {
        if (c == '/') {
            if (!cur.empty()) parts.push_back(cur);
            cur.clear();
        } else {
            cur.push_back(c);
        }
    }
    if (!cur.empty()) parts.push_back(cur);
    return parts;
}
} // namespace detail

Instance* Instance::find_child(const std::string& child_name) const {
    for (const auto& child : children)
        if (child->name == child_name) return child.get();
    return nullptr;
}

void DataModel::reset(const std::string& root_name) {
    root_ = std::make_unique<Instance>();
    root_->referent = alloc_referent();
    root_->class_name = "DataModel";
    root_->name = root_name.empty() ? "Place" : root_name;
    next_referent_ = 2;
    root_->referent = 1;
}

void DataModel::apply_defaults(Instance& inst) const {
    if (const auto* props = reflection_for(inst.class_name)) {
        for (const auto& [name, info] : *props) inst.properties[name] = info.default_value;
    }
}

Instance& DataModel::add_instance(Instance& parent, const std::string& class_name, const std::string& name) {
    auto inst = std::make_unique<Instance>();
    inst->referent = alloc_referent();
    inst->class_name = class_name;
    inst->name = name.empty() ? class_name : name;
    inst->parent = &parent;
    apply_defaults(*inst);
    Instance& ref = *inst;
    parent.children.push_back(std::move(inst));
    return ref;
}

Instance* DataModel::resolve_path(const std::string& path, std::string& error) const {
    auto parts = detail::split_path(path);
    Instance* cur = root_.get();
    size_t i = 0;
    if (!parts.empty() && (parts[0] == "game" || parts[0] == root_->name || parts[0] == root_->class_name)) i = 1;
    for (; i < parts.size(); ++i) {
        Instance* found = nullptr;
        size_t matches = 0;
        for (const auto& child : cur->children) {
            if (child->name == parts[i]) { found = child.get(); ++matches; }
        }
        if (matches == 0) { error = "no instance named '" + parts[i] + "' under '" + path_of(*cur) + "'"; return nullptr; }
        if (matches > 1) { error = "path '" + path + "' is ambiguous: " + std::to_string(matches) + " siblings named '" + parts[i] + "'"; return nullptr; }
        cur = found;
    }
    return cur;
}

std::string DataModel::path_of(const Instance& inst) const {
    if (&inst == root_.get()) return "game";
    std::vector<std::string> parts;
    for (const Instance* cur = &inst; cur && cur != root_.get(); cur = cur->parent) parts.push_back(cur->name);
    std::reverse(parts.begin(), parts.end());
    std::string out;
    for (size_t i = 0; i < parts.size(); ++i) { if (i) out += "/"; out += parts[i]; }
    return out;
}

// Color and BrickColor are aliases of one colour in Roblox: setting
// one updates the other (BrickColor snaps to the nearest palette
// entry). Keep them consistent whenever both are stored.
void sync_color_brick(Instance& inst, const std::string& property) {
    auto color = inst.properties.find("Color");
    auto brick = inst.properties.find("BrickColor");
    if (color == inst.properties.end() || brick == inst.properties.end()) return;
    if (color->second.type != ValueType::Color3 || brick->second.type != ValueType::BrickColor) return;
    if (property == "Color") {
        brick->second.int_value = nearest_brick_color(
            static_cast<float>(color->second.vec3[0]) * 255.0f,
            static_cast<float>(color->second.vec3[1]) * 255.0f,
            static_cast<float>(color->second.vec3[2]) * 255.0f);
    } else if (property == "BrickColor") {
        auto rgb = brick_color_rgb(brick->second.int_value);
        color->second.vec3 = {rgb[0] / 255.0f, rgb[1] / 255.0f, rgb[2] / 255.0f};
    }
}

bool DataModel::set_property(Instance& inst, const std::string& property, const Variant& value, std::string& error) {
    if (property == "Name") {
        if (value.type != ValueType::String) { error = "Name must be a String"; return false; }
        if (value.string_value.empty()) { error = "Name must not be empty"; return false; }
        inst.name = value.string_value;
        return true;
    }
    // Position is the translation component of CFrame: keep the two in sync
    // whenever both are stored (files normally store only CFrame).
    if (property == "Position" && value.type == ValueType::Vector3) {
        inst.properties["Position"] = value;
        auto cf = inst.properties.find("CFrame");
        if (cf != inst.properties.end() && cf->second.type == ValueType::CFrame) {
            cf->second.position = value.vec3;
        }
        return true;
    }
    if (const auto* props = reflection_for(inst.class_name)) {
        auto it = props->find(property);
        if (it != props->end() && it->second.type != value.type) {
            // Reflection-driven numeric conversion (Gate A2): Lua numbers
            // arrive as Float64 and CLI values may be Int64; convert the
            // payload to the reflection type (never just retag, which
            // would lose the value). Non-integral numbers assigned to
            // integer properties are rejected, as in Roblox.
            auto is_int = [](ValueType v) { return v == ValueType::Int32 || v == ValueType::Int64; };
            auto is_float = [](ValueType v) { return v == ValueType::Float32 || v == ValueType::Float64; };
            ValueType expected = it->second.type;
            if ((is_int(expected) || is_float(expected)) && (is_int(value.type) || is_float(value.type))) {
                double dv = is_int(value.type) ? static_cast<double>(value.int_value) : value.float_value;
                Variant coerced;
                if (is_int(expected)) {
                    if (dv != std::floor(dv) || dv < -9.0e15 || dv > 9.0e15) {
                        error = "property '" + property + "' on " + inst.class_name + " expects " +
                                value_type_name(expected) + ", got a non-integral number";
                        return false;
                    }
                    coerced = Variant::make_int64(static_cast<int64_t>(dv));
                } else {
                    coerced = Variant::make_float64(dv);
                }
                coerced.type = expected;
                inst.properties[property] = coerced;
                sync_color_brick(inst, property);
                return true;
            }
            if (expected == ValueType::Enum && value.type == ValueType::EnumItem &&
                value.complex_value.is_object()) {
                Variant coerced = Variant::make_enum(
                    static_cast<int64_t>(value.complex_value.find("value") ? value.complex_value.find("value")->as_double() : 0));
                inst.properties[property] = coerced;
                return true;
            }
            error = "property '" + property + "' on " + inst.class_name + " expects " +
                    value_type_name(it->second.type) + ", got " + value_type_name(value.type);
            return false;
        }
    }
    inst.properties[property] = value;
    if (property == "CFrame") detail::sync_position_from_cframe(inst);
    sync_color_brick(inst, property);
    return true;
}

Json DataModel::instance_to_json(const Instance& inst) {
    Json obj = Json::object();
    obj.set("referent", Json(static_cast<int64_t>(inst.referent)));
    obj.set("className", Json(inst.class_name));
    obj.set("name", Json(inst.name));
    Json props = Json::object();
    for (const auto& [name, value] : inst.properties) props.set(name, value.to_json());
    obj.set("properties", std::move(props));
    Json children = Json::array();
    for (const auto& child : inst.children) children.push(instance_to_json(*child));
    obj.set("children", std::move(children));
    return obj;
}

Json DataModel::snapshot() const { return instance_to_json(*root_); }

std::unique_ptr<Instance> DataModel::instance_from_json(const Json& j, uint64_t& next_ref, std::string& error) {
    const Json* class_j = j.find("className");
    const Json* name_j = j.find("name");
    if (!class_j || !class_j->is_string() || class_j->as_string().empty()) { error = "instance is missing a non-empty 'className'"; return nullptr; }
    auto inst = std::make_unique<Instance>();
    inst->referent = next_ref++;
    inst->class_name = class_j->as_string();
    inst->name = (name_j && name_j->is_string()) ? name_j->as_string() : inst->class_name;
    if (const Json* props = j.find("properties")) {
        if (!props->is_object()) { error = "instance 'properties' must be an object"; return nullptr; }
        for (const auto& [pname, pval] : props->as_object()) {
            if (pname == "Name") continue;
            Variant v;
            if (!Variant::from_json(pval, v, error)) { error = "property '" + pname + "': " + error; return nullptr; }
            inst->properties[pname] = v;
        }
    }
    if (const Json* children = j.find("children")) {
        if (!children->is_array()) { error = "instance 'children' must be an array"; return nullptr; }
        for (const auto& child_j : children->as_array()) {
            auto child = instance_from_json(child_j, next_ref, error);
            if (!child) return nullptr;
            child->parent = inst.get();
            inst->children.push_back(std::move(child));
        }
    }
    return inst;
}

bool DataModel::load_snapshot(const Json& tree, std::string& error) {
    uint64_t next_ref = 1;
    auto new_root = instance_from_json(tree, next_ref, error);
    if (!new_root) return false;
    if (new_root->class_name != "DataModel") { error = "tree root must have className 'DataModel', got '" + new_root->class_name + "'"; return false; }
    root_ = std::move(new_root);
    next_referent_ = next_ref;
    return true;
}

void DataModel::count_rec(const Instance& inst, size_t& n) const {
    ++n;
    for (const auto& child : inst.children) count_rec(*child, n);
}
size_t DataModel::instance_count() const { size_t n = 0; count_rec(*root_, n); return n; }

void DataModel::validate_rec(const Instance& inst, std::vector<Issue>& issues) const {
    std::string path = path_of(inst);
    if (inst.class_name.empty()) issues.push_back({"error", path, "instance has an empty class name"});
    if (inst.name.empty()) issues.push_back({"warning", path, "instance has an empty name"});
    if (!is_known_class(inst.class_name))
        issues.push_back({"warning", path, "unknown class '" + inst.class_name + "' (preserved as-is)"});
    if (const auto* props = reflection_for(inst.class_name)) {
        for (const auto& [pname, value] : inst.properties) {
            auto it = props->find(pname);
            if (it == props->end())
                issues.push_back({"warning", path, "unknown property '" + pname + "' on class " + inst.class_name + " (preserved as-is)"});
            else if (it->second.type != value.type) {
                bool int_pair = (it->second.type == ValueType::Int32 || it->second.type == ValueType::Int64) &&
                                (value.type == ValueType::Int32 || value.type == ValueType::Int64);
                bool float_pair = (it->second.type == ValueType::Float32 || it->second.type == ValueType::Float64) &&
                                  (value.type == ValueType::Float32 || value.type == ValueType::Float64);
                if (!int_pair && !float_pair)
                    issues.push_back({"error", path, "property '" + pname + "' expects " + std::string(value_type_name(it->second.type)) + ", found " + value_type_name(value.type)});
            }
        }
    }
    std::map<std::string, size_t> sibling_names;
    for (const auto& child : inst.children) sibling_names[child->name]++;
    for (const auto& [name, count] : sibling_names)
        if (count > 1)
            issues.push_back({"warning", path, std::to_string(count) + " siblings named '" + name + "'; name paths to them are ambiguous"});
    for (const auto& child : inst.children) validate_rec(*child, issues);
}

std::vector<DataModel::Issue> DataModel::validate() const {
    std::vector<Issue> issues;
    validate_rec(*root_, issues);
    return issues;
}


} // namespace blox
