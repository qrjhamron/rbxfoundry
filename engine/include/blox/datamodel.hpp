#pragma once
#include <cstdint>
#include <map>
#include <memory>
#include <string>
#include <vector>

#include "blox/json.hpp"
#include "blox/variant.hpp"

namespace blox {

struct Instance {
    uint64_t referent = 0;
    std::string class_name;
    std::string name;
    std::map<std::string, Variant> properties; // sorted for deterministic output; excludes Name
    std::vector<std::unique_ptr<Instance>> children;
    Instance* parent = nullptr;

    Instance* find_child(const std::string& child_name) const;
};

// Runtime source of truth for a place. Owns the instance tree rooted at a
// DataModel instance. All mutations go through this class.
class DataModel {
public:
    DataModel() { reset("Place"); }

    void reset(const std::string& root_name);
    Instance& root() { return *root_; }
    const Instance& root() const { return *root_; }

    // Creates an instance (with reflection defaults for known classes),
    // attaches it under `parent`, and returns it.
    Instance& add_instance(Instance& parent, const std::string& class_name, const std::string& name);

    // Resolves "Workspace/Baseplate" style paths. Leading "game", "/" and the
    // root's own name are optional. On failure returns nullptr and sets error;
    // ambiguous sibling names are an error, never a silent first match.
    Instance* resolve_path(const std::string& path, std::string& error) const;
    std::string path_of(const Instance& inst) const;

    // Sets a property, honouring the reflection type for known class/property
    // pairs. Property "Name" renames the instance. Returns false + error.
    bool set_property(Instance& inst, const std::string& property, const Variant& value, std::string& error);

    // ---- Phase 2: hierarchy editing ----
    // Deep-copies `source` (fresh referents) under `new_parent`.
    Instance& clone_instance(const Instance& source, Instance& new_parent, const std::string& name_override);
    // Detaches `inst` (and its subtree) from its parent. Returns subtree size.
    // The root cannot be deleted; checked by callers.
    size_t delete_instance(Instance& inst);
    // Moves `inst` under `new_parent`. Rejects cycles and root moves.
    bool reparent(Instance& inst, Instance& new_parent, std::string& error);
    // Creates a Model under `parent` and moves every target into it.
    Instance& group_instances(std::vector<Instance*> targets, Instance& parent, const std::string& name);
    struct FindCriteria {
        std::string name;       // empty = any
        std::string class_name; // empty = any
        bool substring = false; // name matching mode
    };
    std::vector<Instance*> find(const FindCriteria& criteria, Instance* within) const;

    // ---- Phase 2: transforms (world-space CFrame math) ----
    // Translates a BasePart, or a Model with all its descendant BaseParts.
    bool move(Instance& inst, const std::array<double, 3>& delta, bool relative, std::string& error);
    // Rotates by Euler angles in degrees (intrinsic XYZ order). Parts rotate
    // about their own centre; Models rotate about their pivot.
    bool rotate(Instance& inst, const std::array<double, 3>& angles_deg, bool world_space, std::string& error);
    // Scales a BasePart's Size per-axis, or a Model uniformly about its pivot.
    bool scale(Instance& inst, const std::array<double, 3>& factors, std::string& error);
    // The pivot used for Model transforms: WorldPivot when set (non-identity),
    // otherwise the centre of the descendant BasePart bounding box.
    Variant model_pivot(const Instance& model) const;
    bool set_model_pivot(Instance& model, const Variant& cframe, std::string& error);

    static bool is_base_part(const std::string& class_name);

    Json snapshot() const;
    bool load_snapshot(const Json& tree, std::string& error);

    struct Issue {
        std::string severity; // "error" | "warning"
        std::string path;
        std::string message;
    };
    std::vector<Issue> validate() const;
    size_t instance_count() const;

    static Json instance_to_json(const Instance& inst);
    static std::unique_ptr<Instance> instance_from_json(const Json& j, uint64_t& next_ref, std::string& error);

private:
    std::unique_ptr<Instance> root_;
    uint64_t next_referent_ = 1;
    uint64_t alloc_referent() { return next_referent_++; }
    void apply_defaults(Instance& inst) const;
    void count_rec(const Instance& inst, size_t& n) const;
    void validate_rec(const Instance& inst, std::vector<Issue>& issues) const;
    std::unique_ptr<Instance> clone_rec(const Instance& source, Instance* new_parent);
    void find_rec(const FindCriteria& criteria, Instance& inst, std::vector<Instance*>& out) const;
    void collect_parts(Instance& inst, std::vector<Instance*>& out);
    void collect_models(Instance& inst, std::vector<Instance*>& out);
};

// Reflection for the Phase 1 class subset. Unknown classes/properties are
// preserved verbatim; reflection only drives defaults and type checking.
struct PropertyInfo {
    ValueType type;
    Variant default_value;
};
const std::map<std::string, PropertyInfo>* reflection_for(const std::string& class_name);
bool is_known_class(const std::string& class_name);

} // namespace blox
