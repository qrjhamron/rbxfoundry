#pragma once
#include <array>
#include <memory>
#include <string>
#include <vector>

#include "blox/datamodel/datamodel.hpp"

namespace blox {

// Genuine rigid-body physics (Jolt Physics) over the runtime DataModel,
// plus a capsule character controller for play mode. Bodies are built
// from Workspace BaseParts: Anchored parts are static, unanchored parts
// are dynamic with mass from volume * density. Step is fixed-timestep;
// callers accumulate real time themselves.
class PhysicsWorld {
public:
    struct CharacterInfo {
        bool present = false;
        std::array<double, 3> position{0, 0, 0};
        std::array<double, 3> velocity{0, 0, 0};
        bool grounded = false;
        double health = 100.0;
        std::string state = "Physics"; // HumanoidStateType-like name
        double yaw = 0.0;              // facing, radians
    };

    struct RayHit {
        bool hit = false;
        Instance* instance = nullptr;
        std::string path;
        std::array<double, 3> point{0, 0, 0};
        std::array<double, 3> normal{0, 1, 0};
        double distance = 0.0;
    };

    explicit PhysicsWorld(DataModel& model);
    ~PhysicsWorld();
    PhysicsWorld(const PhysicsWorld&) = delete;
    PhysicsWorld& operator=(const PhysicsWorld&) = delete;

    // Builds bodies for all Workspace BaseParts (idempotent-ish: call
    // once after begin). `character_model` subtree is excluded (driven
    // by the character controller instead).
    void build_world();
    // Spawns (or respawns) the blocky character at the first
    // SpawnLocation, or a default spawn. rig: "R6" | "R15".
    Instance* spawn_character(const std::string& rig);
    // Multi-character API (multiplayer sessions): spawns a named
    // character (or respawns it) and returns its index.
    int spawn_character_multi(const std::string& rig, const std::string& name);
    size_t character_count() const;
    CharacterInfo character_info_at(size_t index) const;
    void set_character_input_at(size_t index, std::array<double, 2> move_xz, bool jump, double yaw);
    Instance* character_model_at(size_t index) const;
    Instance* humanoid_at(size_t index) const;
    // When true, an external animation source (Luau Animator tracks)
    // owns this character's Motor6D Transforms; procedural posing stops
    // overwriting them (the joint chain still evaluates).
    void set_character_anim_driven(size_t index, bool driven);
    std::vector<std::string> take_events_at(size_t index);
    bool has_character() const;
    CharacterInfo character_info() const;
    // Movement input for the next steps: world-space desired direction
    // (unit-ish XZ), jump trigger, camera-relative yaw (radians).
    void set_character_input(std::array<double, 2> move_xz, bool jump, double yaw);
    // One fixed step. Syncs dynamic part CFrames and the character back
    // into the DataModel. Returns instances whose CFrame changed.
    std::vector<Instance*> step(double dt);
    // Raycast against part colliders (respects CanCollide).
    RayHit raycast(std::array<double, 3> origin, std::array<double, 3> direction,
                   double max_distance) const;
    // Filtered raycast: when `allow_only` is non-null, only bodies whose
    // instance appears in that list can be hit (Roblox Include/Exclude
    // filter semantics are resolved to this allowlist by callers).
    RayHit raycast_filtered(std::array<double, 3> origin, std::array<double, 3> direction,
                            double max_distance,
                            const std::vector<Instance*>* allow_only) const;
    // Spatial queries over built bodies (Gate 10). Box is axis-aligned
    // around `center` with full extents `size`; radius is a sphere.
    std::vector<Instance*> parts_in_box(std::array<double, 3> center,
                                        std::array<double, 3> size) const;
    std::vector<Instance*> parts_in_radius(std::array<double, 3> center, double radius) const;
    // Touch begin events since the last call: (touched part, other part).
    std::vector<std::pair<Instance*, Instance*>> take_touch_events();
    double time() const { return time_; }
    size_t body_count() const;
    // Gameplay events since the last call ("died", "respawned").
    std::vector<std::string> take_events();
    Instance* humanoid() const;

    // Implementation state (internal engine class; public for helpers).
    struct Impl;
    std::unique_ptr<Impl> impl_;

private:
    DataModel& model_;
    double time_ = 0.0;
};

} // namespace blox
