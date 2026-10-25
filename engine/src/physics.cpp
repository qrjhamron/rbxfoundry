// Jolt Physics integration: Workspace BaseParts become Jolt bodies and
// are stepped with a fixed timestep, then written back into the runtime
// DataModel. The character is a Jolt CharacterVirtual capsule driving a
// blocky R6/R15 rig. Approximations are deliberate and documented in
// COMPATIBILITY.md (wedge/mesh parts collide as boxes; the character
// does not push dynamic bodies).
#include "blox/physics.hpp"
#include "blox/terrain.hpp"

#include <algorithm>
#include <cmath>
#include <map>
#include <set>
#include <unordered_set>

#include <Jolt/Jolt.h>
#include <Jolt/Core/Factory.h>
#include <Jolt/Core/Memory.h>
#include <Jolt/Core/JobSystemThreadPool.h>
#include <Jolt/Core/TempAllocator.h>
#include <Jolt/Physics/Body/BodyCreationSettings.h>
#include <Jolt/Physics/Character/CharacterVirtual.h>
#include <Jolt/Physics/Collision/CastResult.h>
#include <Jolt/Physics/Collision/RayCast.h>
#include <Jolt/Physics/Collision/Shape/BoxShape.h>
#include <Jolt/Physics/Collision/Shape/CapsuleShape.h>
#include <Jolt/Physics/Collision/Shape/CylinderShape.h>
#include <Jolt/Physics/Collision/Shape/SphereShape.h>
#include <Jolt/Physics/PhysicsSystem.h>
#include <Jolt/Physics/Constraints/FixedConstraint.h>
#include <Jolt/Physics/Constraints/HingeConstraint.h>
#include <Jolt/Physics/Constraints/PointConstraint.h>
#include <Jolt/Physics/Collision/BroadPhase/BroadPhaseLayer.h>
#include <Jolt/Physics/Collision/CollisionGroup.h>
#include <Jolt/Physics/Collision/GroupFilterTable.h>
#include <Jolt/RegisterTypes.h>

using namespace JPH;

namespace blox {
namespace {

namespace Layers {
static constexpr ObjectLayer NON_MOVING = 0;
static constexpr ObjectLayer MOVING = 1;
static constexpr ObjectLayer CHARACTER = 2;
} // namespace Layers

class BPLayerInterface final : public BroadPhaseLayerInterface {
public:
    uint GetNumBroadPhaseLayers() const override { return 3; }
    BroadPhaseLayer GetBroadPhaseLayer(ObjectLayer layer) const override {
        return BroadPhaseLayer(static_cast<BroadPhaseLayer::Type>(layer));
    }
#if defined(JPH_EXTERNAL_PROFILE) || defined(JPH_PROFILE_ENABLED)
    const char* GetBroadPhaseLayerName(BroadPhaseLayer) const override { return "BloxLayer"; }
#endif
};

class ObjectVsBPFilter final : public ObjectVsBroadPhaseLayerFilter {
public:
    bool ShouldCollide(ObjectLayer layer1, BroadPhaseLayer layer2) const override {
        switch (layer1) {
            case Layers::NON_MOVING: return layer2.GetValue() != Layers::NON_MOVING;
            case Layers::MOVING: return true;
            case Layers::CHARACTER: return layer2.GetValue() != Layers::CHARACTER;
            default: return false;
        }
    }
};

class ObjectPairFilter final : public ObjectLayerPairFilter {
public:
    bool ShouldCollide(ObjectLayer a, ObjectLayer b) const override {
        switch (a) {
            case Layers::NON_MOVING: return b == Layers::MOVING || b == Layers::CHARACTER;
            case Layers::MOVING: return true;
            case Layers::CHARACTER: return b == Layers::NON_MOVING || b == Layers::MOVING;
            default: return false;
        }
    }
};

void init_jolt_once() {
    static bool done = [] {
        RegisterDefaultAllocator();
        Factory::sInstance = new Factory();
        RegisterTypes();
        return true;
    }();
    (void)done;
}

const Variant* prop(const Instance& inst, const char* name) {
    auto it = inst.properties.find(name);
    return it == inst.properties.end() ? nullptr : &it->second;
}
std::array<double, 3> prop_vec3(const Instance& inst, const char* name, std::array<double, 3> fallback) {
    const Variant* v = prop(inst, name);
    return (v && v->type == ValueType::Vector3) ? v->vec3 : fallback;
}
double prop_number(const Instance& inst, const char* name, double fallback) {
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
    return (v && (v->type == ValueType::Enum || v->type == ValueType::Int32 || v->type == ValueType::Int64))
               ? v->int_value : fallback;
}

Variant cframe_of(const Instance& inst) {
    const Variant* v = prop(inst, "CFrame");
    return (v && v->type == ValueType::CFrame) ? *v : Variant::make_cframe(0, 0, 0);
}

Quat quat_from_cframe(const Variant& cf) {
    // Basis vectors are the rotation columns.
    Mat44 m(Vec4(static_cast<float>(cf.rotation[0]), static_cast<float>(cf.rotation[3]),
                 static_cast<float>(cf.rotation[6]), 0),
            Vec4(static_cast<float>(cf.rotation[1]), static_cast<float>(cf.rotation[4]),
                 static_cast<float>(cf.rotation[7]), 0),
            Vec4(static_cast<float>(cf.rotation[2]), static_cast<float>(cf.rotation[5]),
                 static_cast<float>(cf.rotation[8]), 0),
            Vec4(0, 0, 0, 1));
    return m.GetQuaternion();
}

void write_cframe(DataModel& model, Instance& inst, Vec3Arg pos, QuatArg rot) {
    Mat44 m = Mat44::sRotation(rot);
    Vec3 c0 = m.GetColumn3(0), c1 = m.GetColumn3(1), c2 = m.GetColumn3(2);
    Variant cf;
    cf.type = ValueType::CFrame;
    cf.position = {pos.GetX(), pos.GetY(), pos.GetZ()};
    cf.rotation = {c0.GetX(), c1.GetX(), c2.GetX(), c0.GetY(), c1.GetY(), c2.GetY(),
                   c0.GetZ(), c1.GetZ(), c2.GetZ()};
    std::string ignored;
    model.set_property(inst, "CFrame", cf, ignored);
}

Mat44 mat44_from_cframe(const Variant& cf) {
    return Mat44::sRotationTranslation(
        quat_from_cframe(cf),
        Vec3(static_cast<float>(cf.position[0]), static_cast<float>(cf.position[1]),
             static_cast<float>(cf.position[2])));
}

Variant cframe_from_mat44(Mat44Arg m) {
    Vec3 c0 = m.GetColumn3(0), c1 = m.GetColumn3(1), c2 = m.GetColumn3(2);
    Vec3 t = m.GetTranslation();
    Variant cf;
    cf.type = ValueType::CFrame;
    cf.position = {t.GetX(), t.GetY(), t.GetZ()};
    cf.rotation = {c0.GetX(), c1.GetX(), c2.GetX(), c0.GetY(), c1.GetY(), c2.GetY(),
                   c0.GetZ(), c1.GetZ(), c2.GetZ()};
    return cf;
}

Variant cframe_prop(const Instance& inst, const char* name) {
    const Variant* v = prop(inst, name);
    return (v && v->type == ValueType::CFrame) ? *v : Variant::make_cframe(0, 0, 0);
}

std::string index_path_of(DataModel& model, Instance* inst) {
    std::string path;
    Instance* cur = inst;
    while (cur && cur != &model.root()) {
        Instance* parent = cur->parent;
        if (!parent) break;
        size_t index = 0;
        bool found = false;
        for (size_t i = 0; i < parent->children.size(); ++i)
            if (parent->children[i].get() == cur) { index = i; found = true; break; }
        if (!found) break;
        path = std::to_string(index) + (path.empty() ? "" : "/" + path);
        cur = parent;
    }
    return path;
}

void set_ref(DataModel& model, Instance& inst, const char* name, Instance* target) {
    Variant v;
    v.type = ValueType::Ref;
    v.string_value = target ? index_path_of(model, target) : "";
    std::string ignored;
    model.set_property(inst, name, v, ignored);
}


} // namespace

// Roblox collision-group matrix filter (Phase 15): pairs keyed by
// Jolt GroupID = Roblox collision-group index.
class CollisionMatrixFilter final : public GroupFilter {
    JPH_DECLARE_SERIALIZABLE_VIRTUAL(JPH_EXPORT, CollisionMatrixFilter)
public:
    std::set<std::pair<uint32, uint32>> disabled;
    bool CanCollide(const CollisionGroup& a, const CollisionGroup& b) const override {
        uint32 x = a.GetGroupID(), y = b.GetGroupID();
        if (x == CollisionGroup::cInvalidGroup || y == CollisionGroup::cInvalidGroup) return true;
        if (x == y) return true;
        return disabled.count({std::min(x, y), std::max(x, y)}) == 0;
    }
    void SaveBinaryState(StreamOut& stream) const override { (void)stream; }
    void RestoreBinaryState(StreamIn& stream) override { (void)stream; }
};

JPH_IMPLEMENT_SERIALIZABLE_VIRTUAL(CollisionMatrixFilter) {
    JPH_ADD_BASE_CLASS(CollisionMatrixFilter, GroupFilter)
}

struct PhysicsWorld::Impl {
    // Declared first so Jolt is fully registered before any allocator or
    // physics object is constructed (members init before the ctor body).
    struct JoltInit {
        JoltInit() { init_jolt_once(); }
    };
    JoltInit jolt_init;
    TempAllocatorImpl allocator{32 * 1024 * 1024};
    JobSystemThreadPool jobs{2048, 8, -1};
    BPLayerInterface bp_interface;
    ObjectVsBPFilter object_vs_bp;
    ObjectPairFilter pair_filter;
    PhysicsSystem physics;
    std::map<Instance*, BodyID> bodies;
    std::map<BodyID, Instance*> by_id;
    BodyFilter body_filter;
    ShapeFilter shape_filter;
    BroadPhaseLayerFilter bp_layer_filter;
    ObjectLayerFilter object_layer_filter;
    CharacterVirtual::ExtendedUpdateSettings update_settings;
    struct CharState {
        Ref<CharacterVirtual> character;
        Instance* model = nullptr;
        Instance* humanoid = nullptr;
        Instance* hrp = nullptr;
        std::vector<Instance*> parts;
        struct Joint {
            Instance* motor = nullptr;
            Instance* part0 = nullptr;
            Instance* part1 = nullptr;
            double swing_sign = 0.0; // procedural walk swing ownership
        };
        std::vector<Joint> joints;
        bool anim_driven = false;
        std::string name;
        std::array<double, 2> move_input{0, 0};
        bool jump_input = false;
        double input_yaw = 0.0;
        double facing_yaw = 0.0;
        double walk_phase = 0.0;
        double vertical_velocity = 0.0;
        double prev_vertical_velocity = 0.0;
        std::vector<std::string> events;
    };
    std::vector<CharState> chars;
    std::vector<Instance*> character_models; // for body-build exclusion
    std::array<double, 3> spawn_point{0, 8, 0};
    double spawn_yaw = 0.0;
    double gravity = 196.2;
    bool built = false;
    std::vector<Ref<Constraint>> constraints;
    Ref<CollisionMatrixFilter> group_filter;
    std::map<std::string, uint32> group_ids;
    std::set<std::pair<BodyID, BodyID>> touching;
    std::vector<std::pair<Instance*, Instance*>> touch_events;

    Impl() {
        init_jolt_once(); // Factory + RegisterTypes must precede Init
        physics.Init(4096, 0, 4096, 8192, bp_interface, object_vs_bp, pair_filter);
    }
};

PhysicsWorld::PhysicsWorld(DataModel& model) : model_(model), impl_(std::make_unique<Impl>()) {
    for (const auto& child : model_.root().children) {
        if (child->class_name == "Workspace") {
            impl_->gravity = prop_number(*child, "Gravity", 196.2);
        }
    }
    impl_->physics.SetGravity(Vec3(0, static_cast<float>(-impl_->gravity), 0));
}

PhysicsWorld::~PhysicsWorld() = default;

Instance* workspace_of(DataModel& model) {
    for (const auto& child : model.root().children) {
        if (child->class_name == "Workspace") return child.get();
    }
    return nullptr;
}


// Resolve a Ref value ("0/2/1" child-index path from the root).
Instance* resolve_ref(DataModel& model, const Variant* v) {
    if (!v || v->type != ValueType::Ref || v->string_value.empty()) return nullptr;
    if (v->string_value.rfind("name:", 0) == 0) {
        Instance* cur = &model.root();
        size_t at = 5;
        const std::string& path = v->string_value;
        while (at <= path.size()) {
            size_t slash = path.find('/', at);
            std::string part = path.substr(at, slash == std::string::npos ? slash : slash - at);
            Instance* next = nullptr;
            for (const auto& child : cur->children)
                if (child->name == part) { next = child.get(); break; }
            if (!next) return nullptr;
            cur = next;
            if (slash == std::string::npos) break;
            at = slash + 1;
        }
        return cur;
    }
    Instance* cur = &model.root();
    size_t at = 0;
    const std::string& path = v->string_value;
    while (at <= path.size()) {
        size_t slash = path.find('/', at);
        std::string part = path.substr(at, slash == std::string::npos ? slash : slash - at);
        char* end = nullptr;
        long idx = std::strtol(part.c_str(), &end, 10);
        if (end == part.c_str() || idx < 0 || static_cast<size_t>(idx) >= cur->children.size())
            return nullptr;
        cur = cur->children[static_cast<size_t>(idx)].get();
        if (slash == std::string::npos) break;
        at = slash + 1;
    }
    return cur;
}

// World CFrame (as Mat44) of an Attachment: parent part CFrame * local.
Mat44 attachment_world(DataModel& model, Instance* attachment) {
    Instance* part = attachment->parent;
    Variant local = cframe_of(*attachment);
    Mat44 local_m = Mat44::sRotationTranslation(
        quat_from_cframe(local),
        Vec3(static_cast<float>(local.position[0]), static_cast<float>(local.position[1]),
             static_cast<float>(local.position[2])));
    if (!part) return local_m;
    Variant pcf = cframe_of(*part);
    Mat44 part_m = Mat44::sRotationTranslation(
        quat_from_cframe(pcf),
        Vec3(static_cast<float>(pcf.position[0]), static_cast<float>(pcf.position[1]),
             static_cast<float>(pcf.position[2])));
    (void)model;
    return part_m * local_m;
}

void build_constraints(DataModel& model, PhysicsWorld::Impl& im) {
    Instance* workspace = workspace_of(model);
    if (!workspace) return;
    std::vector<Instance*> stack{workspace};
    std::vector<Instance*> all;
    while (!stack.empty()) {
        Instance* cur = stack.back();
        stack.pop_back();
        all.push_back(cur);
        for (const auto& child : cur->children) stack.push_back(child.get());
    }
    BodyInterface& bodies = im.physics.GetBodyInterfaceNoLock();
    auto body_of = [&](Instance* inst) -> BodyID {
        auto it = im.bodies.find(inst);
        return it == im.bodies.end() ? BodyID() : it->second;
    };
    for (Instance* inst : all) {
        if (inst->class_name == "WeldConstraint") {
            Instance* p0 = resolve_ref(model, prop(*inst, "Part0"));
            Instance* p1 = resolve_ref(model, prop(*inst, "Part1"));
            if (!p0 || !p1) continue;
            BodyID b0 = body_of(p0), b1 = body_of(p1);
            if (b0.IsInvalid() || b1.IsInvalid()) continue;
            FixedConstraintSettings settings;
            settings.mSpace = EConstraintSpace::WorldSpace;
            Variant cf = cframe_of(*p0);
            settings.mPoint1 = settings.mPoint2 =
                Vec3(static_cast<float>(cf.position[0]), static_cast<float>(cf.position[1]),
                     static_cast<float>(cf.position[2]));
            settings.mAxisX1 = settings.mAxisX2 = Vec3::sAxisX();
            settings.mAxisY1 = settings.mAxisY2 = Vec3::sAxisY();
            Body* body0 = nullptr;
            Body* body1 = nullptr;
            {
                BodyLockWrite lock(im.physics.GetBodyLockInterface(), b0);
                BodyLockWrite lock2(im.physics.GetBodyLockInterface(), b1);
                body0 = &lock.GetBody();
                body1 = &lock2.GetBody();
            }
            auto* constraint = new FixedConstraint(*body0, *body1, settings);
            im.physics.AddConstraint(constraint);
            im.constraints.push_back(constraint);
        } else if (inst->class_name == "BallSocketConstraint" ||
                   inst->class_name == "HingeConstraint") {
            Instance* a0 = resolve_ref(model, prop(*inst, "Attachment0"));
            Instance* a1 = resolve_ref(model, prop(*inst, "Attachment1"));
            if (!a0 || !a1 || !a0->parent || !a1->parent) continue;
            BodyID b0 = body_of(a0->parent), b1 = body_of(a1->parent);
            if (b0.IsInvalid() || b1.IsInvalid()) continue;
            Mat44 w0 = attachment_world(model, a0);
            Mat44 w1 = attachment_world(model, a1);
            Vec3 point = w0.GetTranslation();
            if (inst->class_name == "BallSocketConstraint") {
                PointConstraintSettings settings;
                settings.mSpace = EConstraintSpace::WorldSpace;
                settings.mPoint1 = point;
                settings.mPoint2 = w1.GetTranslation();
                Body* body0 = nullptr;
                Body* body1 = nullptr;
                {
                    BodyLockWrite lock(im.physics.GetBodyLockInterface(), b0);
                    BodyLockWrite lock2(im.physics.GetBodyLockInterface(), b1);
                    body0 = &lock.GetBody();
                    body1 = &lock2.GetBody();
                }
                auto* constraint = new PointConstraint(*body0, *body1, settings);
                im.physics.AddConstraint(constraint);
                im.constraints.push_back(constraint);
            } else {
                HingeConstraintSettings settings;
                settings.mSpace = EConstraintSpace::WorldSpace;
                settings.mPoint1 = point;
                settings.mPoint2 = w1.GetTranslation();
                // Hinge axis = Attachment0 world X axis (Roblox Attachment
                // Axis default), normal = world Z axis.
                settings.mHingeAxis1 = settings.mHingeAxis2 = w0.GetColumn3(0).Normalized();
                settings.mNormalAxis1 = settings.mNormalAxis2 = w0.GetColumn3(2).Normalized();
                Body* body0 = nullptr;
                Body* body1 = nullptr;
                {
                    BodyLockWrite lock(im.physics.GetBodyLockInterface(), b0);
                    BodyLockWrite lock2(im.physics.GetBodyLockInterface(), b1);
                    body0 = &lock.GetBody();
                    body1 = &lock2.GetBody();
                }
                auto* constraint = new HingeConstraint(*body0, *body1, settings);
                im.physics.AddConstraint(constraint);
                im.constraints.push_back(constraint);
            }
        }
    }
}

void PhysicsWorld::build_world() {
    if (impl_->built) return;
    impl_->built = true;
    Instance* workspace = workspace_of(model_);
    if (!workspace) return;
    // Terrain voxel collision: static boxes for voxels exposed to air.
    if (Instance* terrain = find_terrain(model_)) {
        TerrainGrid grid;
        std::string terr_error;
        if (terrain_load(*terrain, grid, terr_error) && !grid.empty()) {
            BodyInterface& bodies = impl_->physics.GetBodyInterface();
            RefConst<Shape> voxel_shape = new BoxShape(Vec3(
                static_cast<float>(kVoxelSize / 2), static_cast<float>(kVoxelSize / 2),
                static_cast<float>(kVoxelSize / 2)));
            for (const auto& [key, value] : grid.cells) {
                auto [x, y, z] = key;
                bool exposed = !grid.solid(x, y + 1, z) || !grid.solid(x + 1, y, z) ||
                               !grid.solid(x - 1, y, z) || !grid.solid(x, y, z + 1) ||
                               !grid.solid(x, y, z - 1) || !grid.solid(x, y - 1, z);
                if (!exposed) continue;
                auto centre = voxel_center(x, y, z);
                BodyCreationSettings settings(
                    voxel_shape,
                    Vec3(static_cast<float>(centre[0]), static_cast<float>(centre[1]),
                         static_cast<float>(centre[2])),
                    Quat::sIdentity(), EMotionType::Static, Layers::NON_MOVING);
                BodyID id = bodies.CreateAndAddBody(settings, EActivation::DontActivate);
                if (!id.IsInvalid()) {
                    impl_->bodies[terrain] = id;
                    impl_->by_id[id] = terrain;
                }
            }
        }
    }
    // Collision groups (Phase 15): names from BasePart CollisionGroup
    // properties; non-collidable pairs from PhysicsService attributes
    // written by Luau before physics begins.
    {
        std::vector<Instance*> scan{workspace};
        while (!scan.empty()) {
            Instance* cur = scan.back();
            scan.pop_back();
            if (DataModel::is_base_part(cur->class_name)) {
                const Variant* g = prop(*cur, "CollisionGroup");
                std::string name = (g && g->type == ValueType::String && !g->string_value.empty())
                                       ? g->string_value : "Default";
                if (!impl_->group_ids.count(name))
                    impl_->group_ids[name] = static_cast<uint32>(impl_->group_ids.size());
            }
            for (const auto& child : cur->children) scan.push_back(child.get());
        }
        if (!impl_->group_ids.count("Default")) impl_->group_ids["Default"] = 0;
        impl_->group_filter = new CollisionMatrixFilter();
        for (const auto& svc : model_.root().children) {
            if (svc->class_name != "PhysicsService") continue;
            auto it = svc->properties.find("Attributes");
            if (it == svc->properties.end() || it->second.type != ValueType::Attributes) continue;
            for (const auto& [key, entry] : it->second.complex_value.as_object()) {
                if (key.rfind("cg:", 0) != 0) continue;
                if (!entry.find("value") || entry.find("value")->as_bool()) continue;
                std::string pair = key.substr(3);
                size_t bar = pair.find('|');
                if (bar == std::string::npos) continue;
                auto ia = impl_->group_ids.find(pair.substr(0, bar));
                auto ib = impl_->group_ids.find(pair.substr(bar + 1));
                if (ia == impl_->group_ids.end() || ib == impl_->group_ids.end()) continue;
                impl_->group_filter->disabled.insert(
                    {std::min(ia->second, ib->second), std::max(ia->second, ib->second)});
            }
        }
    }
    BodyInterface& bodies = impl_->physics.GetBodyInterface();
    std::vector<Instance*> stack{workspace};
    while (!stack.empty()) {
        Instance* inst = stack.back();
        stack.pop_back();
        for (const auto& child : inst->children) stack.push_back(child.get());
        if (inst == workspace || !DataModel::is_base_part(inst->class_name)) continue;
        {
            bool in_character = false;
            for (const Instance* cur = inst; cur; cur = cur->parent) {
                for (const Instance* model : impl_->character_models) {
                    if (cur == model) in_character = true;
                }
            }
            if (in_character) continue;
        }
        if (!prop_bool(*inst, "CanCollide", true)) continue;
        auto size = prop_vec3(*inst, "Size", {4, 1, 2});
        Variant cf = cframe_of(*inst);
        RefConst<Shape> shape;
        Quat extra = Quat::sIdentity();
        if (inst->class_name == "Part" && prop_enum(*inst, "Shape", 1) == 0) {
            shape = new SphereShape(static_cast<float>(std::min({size[0], size[1], size[2]}) / 2.0));
        } else if (inst->class_name == "Part" && prop_enum(*inst, "Shape", 1) == 2) {
            shape = new CylinderShape(static_cast<float>(size[0] / 2.0),
                                      static_cast<float>(std::min(size[1], size[2]) / 2.0));
            extra = Quat::sRotation(Vec3::sAxisZ(), -1.5707963f); // Jolt Y axis -> Roblox X axis
        } else {
            // Block, WedgePart, CornerWedgePart, MeshPart, SpawnLocation:
            // box collider (documented approximation for wedges/meshes).
            shape = new BoxShape(Vec3(static_cast<float>(size[0] / 2), static_cast<float>(size[1] / 2),
                                      static_cast<float>(size[2] / 2)));
        }
        bool anchored = prop_bool(*inst, "Anchored", false);
        BodyCreationSettings settings(
            shape, Vec3(static_cast<float>(cf.position[0]), static_cast<float>(cf.position[1]),
                        static_cast<float>(cf.position[2])),
            quat_from_cframe(cf) * extra,
            anchored ? EMotionType::Static : EMotionType::Dynamic,
            anchored ? Layers::NON_MOVING : Layers::MOVING);
        {
            const Variant* g = prop(*inst, "CollisionGroup");
            std::string name = (g && g->type == ValueType::String && !g->string_value.empty())
                                   ? g->string_value : "Default";
            auto git = impl_->group_ids.find(name);
            uint32 gid = git != impl_->group_ids.end() ? git->second : 0;
            settings.mCollisionGroup = CollisionGroup(impl_->group_filter, gid, 0);
        }
        if (!anchored) {
            settings.mMotionQuality = EMotionQuality::LinearCast;
            // Mass from volume * density (Roblox-like): honour
            // CustomPhysicalProperties density when present.
            double density = 1.0;
            if (const Variant* phys = prop(*inst, "CustomPhysicalProperties")) {
                if (phys->complex_value.is_object()) {
                    if (const Json* d = phys->complex_value.find("density")) density = d->as_double(density);
                }
            }
            double volume = std::max(0.001, size[0] * size[1] * size[2]);
            settings.mOverrideMassProperties = EOverrideMassProperties::CalculateInertia;
            settings.mMassPropertiesOverride.mMass = static_cast<float>(std::max(0.05, volume * density));
            settings.mFriction = 0.5f;
            settings.mRestitution = 0.0f;
        } else {
            settings.mFriction = 0.5f;
        }
        BodyID id = bodies.CreateAndAddBody(settings, EActivation::Activate);
        if (!id.IsInvalid()) {
            impl_->bodies[inst] = id;
            impl_->by_id[id] = inst;
        }
    }
    build_constraints(model_, *impl_);
}

std::vector<Instance*> PhysicsWorld::parts_in_box(std::array<double, 3> center,
                                                  std::array<double, 3> size) const {
    std::vector<Instance*> out;
    AABox box(Vec3(static_cast<float>(center[0] - size[0] / 2), static_cast<float>(center[1] - size[1] / 2),
                   static_cast<float>(center[2] - size[2] / 2)),
              Vec3(static_cast<float>(center[0] + size[0] / 2), static_cast<float>(center[1] + size[1] / 2),
                   static_cast<float>(center[2] + size[2] / 2)));
    class Collector final : public CollideShapeBodyCollector {
    public:
        void AddHit(const BodyID& id) override { ids.push_back(id); }
        std::vector<BodyID> ids;
    } collector;
    impl_->physics.GetBroadPhaseQuery().CollideAABox(box, collector, impl_->bp_layer_filter,
                                                     impl_->object_layer_filter);
    for (BodyID id : collector.ids) {
        auto it = impl_->by_id.find(id);
        if (it != impl_->by_id.end()) out.push_back(it->second);
    }
    return out;
}

std::vector<Instance*> PhysicsWorld::parts_in_radius(std::array<double, 3> center, double radius) const {
    std::vector<Instance*> out;
    class Collector final : public CollideShapeBodyCollector {
    public:
        void AddHit(const BodyID& id) override { ids.push_back(id); }
        std::vector<BodyID> ids;
    } collector;
    impl_->physics.GetBroadPhaseQuery().CollideSphere(
        Vec3(static_cast<float>(center[0]), static_cast<float>(center[1]),
             static_cast<float>(center[2])),
        static_cast<float>(radius), collector, impl_->bp_layer_filter, impl_->object_layer_filter);
    for (BodyID id : collector.ids) {
        auto it = impl_->by_id.find(id);
        if (it != impl_->by_id.end()) out.push_back(it->second);
    }
    return out;
}

std::vector<std::pair<Instance*, Instance*>> PhysicsWorld::take_touch_events() {
    auto out = std::move(impl_->touch_events);
    impl_->touch_events.clear();
    return out;
}

size_t PhysicsWorld::body_count() const { return impl_->bodies.size(); }

std::vector<std::string> PhysicsWorld::take_events() { return take_events_at(0); }

std::vector<std::string> PhysicsWorld::take_events_at(size_t index) {
    if (index >= impl_->chars.size()) return {};
    std::vector<std::string> out = impl_->chars[index].events;
    impl_->chars[index].events.clear();
    return out;
}

void PhysicsWorld::set_character_anim_driven(size_t index, bool driven) {
    if (index < impl_->chars.size()) impl_->chars[index].anim_driven = driven;
}

Instance* PhysicsWorld::humanoid() const { return humanoid_at(0); }

Instance* PhysicsWorld::humanoid_at(size_t index) const {
    return index < impl_->chars.size() ? impl_->chars[index].humanoid : nullptr;
}

Instance* PhysicsWorld::character_model_at(size_t index) const {
    return index < impl_->chars.size() ? impl_->chars[index].model : nullptr;
}

size_t PhysicsWorld::character_count() const { return impl_->chars.size(); }

bool PhysicsWorld::has_character() const { return !impl_->chars.empty(); }

// ------------------------------------------------------------- character --

Instance* make_part(DataModel& model, Instance& parent, const std::string& name,
                    std::array<double, 3> size, std::array<double, 3> color) {
    Instance& part = model.add_instance(parent, "Part", name);
    std::string ignored;
    Variant size_v = Variant::make_vec3(size[0], size[1], size[2]);
    model.set_property(part, "Size", size_v, ignored);
    model.set_property(part, "Color", Variant::make_color3(color[0], color[1], color[2]), ignored);
    model.set_property(part, "Anchored", Variant::make_bool(false), ignored);
    model.set_property(part, "CanCollide", Variant::make_bool(false), ignored);
    return &part;
}

int PhysicsWorld::spawn_character_multi(const std::string& rig, const std::string& name) {
    Impl& im = *impl_;
    Instance* workspace = workspace_of(model_);
    if (!workspace) return -1;
    // Respawn an existing character with this name.
    for (size_t i = 0; i < im.chars.size(); ++i) {
        if (im.chars[i].name == name) {
            Impl::CharState& ch = im.chars[i];
            ch.character->SetPosition(Vec3(static_cast<float>(im.spawn_point[0]),
                                           static_cast<float>(im.spawn_point[1]),
                                           static_cast<float>(im.spawn_point[2])));
            ch.vertical_velocity = 0;
            if (ch.humanoid) {
                std::string ignored;
                model_.set_property(*ch.humanoid, "Health", Variant::make_float64(100.0), ignored);
            }
            ch.events.push_back("respawned");
            return static_cast<int>(i);
        }
    }
    Impl::CharState ch;
    ch.name = name;
    Instance& model_inst = model_.add_instance(*workspace, "Model", name);
    ch.model = &model_inst;
    im.character_models.push_back(&model_inst);
    Instance& humanoid = model_.add_instance(model_inst, "Humanoid", "Humanoid");
    ch.humanoid = &humanoid;
    auto set_num = [&](Instance& inst, const char* prop, double v) {
        std::string ignored;
        model_.set_property(inst, prop, Variant::make_float64(v), ignored);
    };
    set_num(humanoid, "WalkSpeed", 16.0);
    set_num(humanoid, "JumpPower", 50.0);
    set_num(humanoid, "Health", 100.0);
    set_num(humanoid, "MaxHealth", 100.0);
    {
        std::string ignored;
        model_.set_property(humanoid, "RigType", Variant::make_enum(rig == "R15" ? 1 : 0), ignored);
    }

    const std::array<double, 3> skin{1.0, 0.85, 0.35};   // classic yellow
    const std::array<double, 3> torso_c{0.05, 0.55, 0.65};
    const std::array<double, 3> legs_c{0.55, 0.75, 0.25};
    Instance* hrp = make_part(model_, model_inst, "HumanoidRootPart", {2, 2, 1}, torso_c);
    ch.hrp = hrp;
    {
        std::string ignored;
        model_.set_property(*hrp, "Transparency", Variant::make_float64(1.0), ignored);
    }
    ch.parts.push_back(hrp);
    std::map<std::string, std::array<double, 3>> part_sizes;
    part_sizes["HumanoidRootPart"] = {2, 2, 1};
    auto add = [&](const char* part_name, std::array<double, 3> size, std::array<double, 3> color) {
        Instance* part = make_part(model_, model_inst, part_name, size, color);
        ch.parts.push_back(part);
        part_sizes[part_name] = size;
        return part;
    };
    if (rig == "R15") {
        add("LowerTorso", {1.8, 0.9, 0.9}, torso_c);
        add("UpperTorso", {1.8, 1.1, 0.9}, torso_c);
        add("Head", {1.2, 1.2, 1.2}, skin);
        add("LeftUpperArm", {0.8, 0.9, 0.8}, skin);
        add("LeftLowerArm", {0.7, 0.8, 0.7}, skin);
        add("LeftHand", {0.6, 0.5, 0.6}, skin);
        add("RightUpperArm", {0.8, 0.9, 0.8}, skin);
        add("RightLowerArm", {0.7, 0.8, 0.7}, skin);
        add("RightHand", {0.6, 0.5, 0.6}, skin);
        add("LeftUpperLeg", {0.9, 0.9, 0.9}, legs_c);
        add("LeftLowerLeg", {0.8, 0.8, 0.8}, legs_c);
        add("LeftFoot", {0.7, 0.4, 0.9}, legs_c);
        add("RightUpperLeg", {0.9, 0.9, 0.9}, legs_c);
        add("RightLowerLeg", {0.8, 0.8, 0.8}, legs_c);
        add("RightFoot", {0.7, 0.4, 0.9}, legs_c);
    } else {
        add("Torso", {2, 2, 1}, torso_c);
        add("Head", {1.2, 1.2, 1.2}, skin);
        add("Left Arm", {1, 2, 1}, skin);
        add("Right Arm", {1, 2, 1}, skin);
        add("Left Leg", {1, 2, 1}, legs_c);
        add("Right Leg", {1, 2, 1}, legs_c);
    }
    // Animator + Motor6D rig: parts are placed at rest, then Motor6D
    // joints (C0/C1 from the rest pose, pivots at the top of Part1) are
    // created as children of Part0, exactly like a Roblox rig.
    model_.add_instance(humanoid, "Animator", "Animator");
    {
        auto part_by_name = [&](const std::string& pname) -> Instance* {
            for (Instance* part : ch.parts)
                if (part->name == pname) return part;
            return nullptr;
        };
        // Rest offsets from the capsule centre (matches pose layout).
        static const std::map<std::string, std::array<double, 3>> kRest = {
            {"HumanoidRootPart", {0, -0.5, 0}}, {"Torso", {0, 0.4, 0}},
            {"LowerTorso", {0, -0.1, 0}}, {"UpperTorso", {0, 0.8, 0}}, {"Head", {0, 1.9, 0}},
            {"Left Arm", {1.5, 0.4, 0}}, {"Right Arm", {-1.5, 0.4, 0}},
            {"LeftUpperArm", {1.4, 0.7, 0}}, {"LeftLowerArm", {1.4, -0.1, 0}}, {"LeftHand", {1.4, -0.7, 0}},
            {"RightUpperArm", {-1.4, 0.7, 0}}, {"RightLowerArm", {-1.4, -0.1, 0}}, {"RightHand", {-1.4, -0.7, 0}},
            {"Left Leg", {0.5, -1.6, 0}}, {"Right Leg", {-0.5, -1.6, 0}},
            {"LeftUpperLeg", {0.5, -1.0, 0}}, {"LeftLowerLeg", {0.5, -1.8, 0}}, {"LeftFoot", {0.5, -2.3, 0}},
            {"RightUpperLeg", {-0.5, -1.0, 0}}, {"RightLowerLeg", {-0.5, -1.8, 0}}, {"RightFoot", {-0.5, -2.3, 0}},
        };
        double slot = static_cast<double>(im.chars.size()) * 3.0;
        Vec3 centre(static_cast<float>(im.spawn_point[0] + slot),
                    static_cast<float>(im.spawn_point[1]), static_cast<float>(im.spawn_point[2]));
        Quat facing = Quat::sRotation(Vec3::sAxisY(), static_cast<float>(im.spawn_yaw));
        Mat44 facing_m = Mat44::sRotation(facing);
        Vec3 right = facing_m.GetColumn3(0);
        Vec3 back = facing_m.GetColumn3(2);
        std::map<Instance*, Mat44> rest_world;
        for (Instance* part : ch.parts) {
            auto it = kRest.find(part->name);
            if (it == kRest.end()) continue;
            Vec3 pos = centre + right * static_cast<float>(it->second[0]) +
                       Vec3(0, static_cast<float>(it->second[1]), 0) +
                       back * static_cast<float>(it->second[2]);
            write_cframe(model_, *part, pos, facing);
            rest_world[part] = Mat44::sRotationTranslation(facing, pos);
        }
        struct JointDef { const char* joint; const char* p0; const char* p1; double swing; };
        static const JointDef kR6[] = {
            {"RootJoint", "HumanoidRootPart", "Torso", 0},
            {"Neck", "Torso", "Head", 0},
            {"Left Shoulder", "Torso", "Left Arm", 1},
            {"Right Shoulder", "Torso", "Right Arm", -1},
            {"Left Hip", "Torso", "Left Leg", -1},
            {"Right Hip", "Torso", "Right Leg", 1},
        };
        static const JointDef kR15[] = {
            {"Root", "HumanoidRootPart", "LowerTorso", 0},
            {"Waist", "LowerTorso", "UpperTorso", 0},
            {"Neck", "UpperTorso", "Head", 0},
            {"Left Shoulder", "UpperTorso", "LeftUpperArm", 1},
            {"Left Elbow", "LeftUpperArm", "LeftLowerArm", 0.3},
            {"Left Wrist", "LeftLowerArm", "LeftHand", 0},
            {"Right Shoulder", "UpperTorso", "RightUpperArm", -1},
            {"Right Elbow", "RightUpperArm", "RightLowerArm", -0.3},
            {"Right Wrist", "RightLowerArm", "RightHand", 0},
            {"Left Hip", "LowerTorso", "LeftUpperLeg", -1},
            {"Left Knee", "LeftUpperLeg", "LeftLowerLeg", -0.3},
            {"Left Ankle", "LeftLowerLeg", "LeftFoot", 0},
            {"Right Hip", "LowerTorso", "RightUpperLeg", 1},
            {"Right Knee", "RightUpperLeg", "RightLowerLeg", 0.3},
            {"Right Ankle", "RightLowerLeg", "RightFoot", 0},
        };
        const JointDef* defs = rig == "R15" ? kR15 : kR6;
        size_t def_count = rig == "R15" ? std::size(kR15) : std::size(kR6);
        for (size_t i = 0; i < def_count; ++i) {
            Instance* p0 = part_by_name(defs[i].p0);
            Instance* p1 = part_by_name(defs[i].p1);
            if (!p0 || !p1) continue;
            Instance& motor = model_.add_instance(*p0, "Motor6D", defs[i].joint);
            set_ref(model_, motor, "Part0", p0);
            set_ref(model_, motor, "Part1", p1);
            double half_h = part_sizes[defs[i].p1][1] / 2.0;
            Mat44 c1 = Mat44::sTranslation(Vec3(0, static_cast<float>(half_h), 0));
            Mat44 joint_world = rest_world[p1] * c1;
            Mat44 c0 = rest_world[p0].Inversed() * joint_world;
            std::string ignored;
            model_.set_property(motor, "C0", cframe_from_mat44(c0), ignored);
            model_.set_property(motor, "C1", cframe_from_mat44(c1), ignored);
            ch.joints.push_back({&motor, p0, p1, defs[i].swing});
        }
    }
    CharacterVirtualSettings settings;
    settings.mMaxSlopeAngle = DegreesToRadians(50.0f);
    settings.mMaxStrength = 100.0f;
    settings.mShape = new CapsuleShape(1.5f, 1.0f); // 5 studs tall, 2 wide
    settings.mSupportingVolume = Plane(Vec3::sAxisY(), -1.0f);
    // Stagger simultaneous spawns slightly so capsules don't overlap.
    double slot = static_cast<double>(im.chars.size()) * 3.0;
    ch.character = new CharacterVirtual(&settings,
                                        Vec3(static_cast<float>(im.spawn_point[0] + slot),
                                             static_cast<float>(im.spawn_point[1]),
                                             static_cast<float>(im.spawn_point[2])),
                                        Quat::sRotation(Vec3::sAxisY(), static_cast<float>(im.spawn_yaw)),
                                        0, &im.physics);
    ch.facing_yaw = im.spawn_yaw;
    ch.vertical_velocity = 0.0;
    im.chars.push_back(std::move(ch));
    return static_cast<int>(im.chars.size() - 1);
}

Instance* PhysicsWorld::spawn_character(const std::string& rig) {
    Impl& im = *impl_;
    Instance* workspace = workspace_of(model_);
    if (!workspace) return nullptr;
    // Find spawn (shared by all characters).
    std::vector<Instance*> stack{workspace};
    Instance* spawn = nullptr;
    while (!stack.empty() && !spawn) {
        Instance* cur = stack.back();
        stack.pop_back();
        if (cur->class_name == "SpawnLocation") spawn = cur;
        for (const auto& child : cur->children) stack.push_back(child.get());
    }
    if (spawn) {
        Variant cf = cframe_of(*spawn);
        im.spawn_point = {cf.position[0], cf.position[1] + 3.0, cf.position[2]};
        // Facing = LookVector (-rotation column 3); yaw follows the
        // convention forward = (-sin yaw, -cos yaw).
        im.spawn_yaw = std::atan2(cf.rotation[2], cf.rotation[8]);
    }
    int index = spawn_character_multi(rig, "Player");
    return index >= 0 ? im.chars[static_cast<size_t>(index)].model : nullptr;
}

void PhysicsWorld::set_character_input(std::array<double, 2> move_xz, bool jump, double yaw) {
    set_character_input_at(0, move_xz, jump, yaw);
}

void PhysicsWorld::set_character_input_at(size_t index, std::array<double, 2> move_xz, bool jump,
                                          double yaw) {
    if (index >= impl_->chars.size()) return;
    impl_->chars[index].move_input = move_xz;
    impl_->chars[index].jump_input = jump;
    impl_->chars[index].input_yaw = yaw;
}

PhysicsWorld::CharacterInfo PhysicsWorld::character_info() const { return character_info_at(0); }

PhysicsWorld::CharacterInfo PhysicsWorld::character_info_at(size_t index) const {
    CharacterInfo info;
    const Impl& im = *impl_;
    if (index >= im.chars.size()) return info;
    const Impl::CharState& ch = im.chars[index];
    info.present = true;
    Vec3 pos = ch.character->GetPosition();
    Vec3 vel = ch.character->GetLinearVelocity();
    info.position = {pos.GetX(), pos.GetY(), pos.GetZ()};
    info.velocity = {vel.GetX(), vel.GetY(), vel.GetZ()};
    info.grounded = ch.character->GetGroundState() == CharacterBase::EGroundState::OnGround ||
                    ch.character->GetGroundState() == CharacterBase::EGroundState::OnSteepGround;
    info.yaw = ch.facing_yaw;
    if (ch.humanoid) info.health = prop_number(*ch.humanoid, "Health", 100.0);
    double speed = std::sqrt(vel.GetX() * vel.GetX() + vel.GetZ() * vel.GetZ());
    if (info.health <= 0) info.state = "Dead";
    else if (!info.grounded) info.state = vel.GetY() > 1.0 ? "Jumping" : "Freefall";
    else if (speed > 0.5) info.state = "Running";
    else info.state = "Running"; // Humanoid reports Running even idle-ish
    return info;
}

// Pose the rig around the capsule. When the character has a Motor6D
// rig, posing evaluates the real joint chain
// (Part1 = Part0 * C0 * Transform * C1^-1); procedural walk/idle
// Transforms are written by physics unless a Luau AnimationTrack owns
// them (anim_driven). Without joints, falls back to direct placement.
void pose_character(PhysicsWorld::Impl& im, DataModel& model, PhysicsWorld::Impl::CharState& ch,
                    Vec3Arg char_pos, double dt) {
    Vec3 vel = ch.character->GetLinearVelocity();
    double speed = std::sqrt(static_cast<double>(vel.GetX() * vel.GetX() + vel.GetZ() * vel.GetZ()));
    bool grounded = ch.character->GetGroundState() == CharacterBase::EGroundState::OnGround;
    if (grounded && speed > 0.5) ch.walk_phase += dt * speed * 1.6;
    double swing = std::sin(ch.walk_phase) * (grounded ? std::min(1.0, speed / 16.0) : 0.2);
    double feet_y = char_pos.GetY() - 2.5;
    Quat facing = Quat::sRotation(Vec3::sAxisY(), static_cast<float>(ch.facing_yaw));
    Mat44 facing_m = Mat44::sRotation(facing);
    Vec3 right = facing_m.GetColumn3(0);
    Vec3 back = facing_m.GetColumn3(2);
    Vec3 centre = char_pos;
    (void)im;
    if (!ch.joints.empty() && ch.hrp) {
        // Procedural Transforms (skipped while a script track drives).
        if (!ch.anim_driven) {
            for (const auto& joint : ch.joints) {
                double angle;
                if (joint.swing_sign == 0.0) angle = 0.0;
                else if (std::abs(joint.swing_sign) < 0.9)
                    angle = joint.swing_sign * (0.35 + 0.3 * swing); // elbows/knees keep a bend
                else
                    angle = swing * joint.swing_sign * 0.7;
                Variant t;
                t.type = ValueType::CFrame;
                if (angle != 0.0) {
                    Mat44 rot = Mat44::sRotation(Quat::sRotation(Vec3::sAxisX(), static_cast<float>(angle)));
                    Vec3 c0 = rot.GetColumn3(0), c1 = rot.GetColumn3(1), c2 = rot.GetColumn3(2);
                    t.rotation = {c0.GetX(), c1.GetX(), c2.GetX(), c0.GetY(), c1.GetY(), c2.GetY(),
                                  c0.GetZ(), c1.GetZ(), c2.GetZ()};
                }
                std::string ignored;
                model.set_property(*joint.motor, "Transform", t, ignored);
            }
        }
        // Joint-chain evaluation from the HumanoidRootPart.
        std::map<Instance*, Mat44> world;
        Mat44 hrp_world = Mat44::sRotationTranslation(
            facing, centre + Vec3(0, -0.5f, 0));
        world[ch.hrp] = hrp_world;
        write_cframe(model, *ch.hrp, hrp_world.GetTranslation(), facing);
        for (const auto& joint : ch.joints) {
            auto it = world.find(joint.part0);
            if (it == world.end()) continue;
            Mat44 joint_world = it->second * mat44_from_cframe(cframe_prop(*joint.motor, "C0"));
            Mat44 part1_world = joint_world * mat44_from_cframe(cframe_prop(*joint.motor, "Transform")) *
                                mat44_from_cframe(cframe_prop(*joint.motor, "C1")).Inversed();
            world[joint.part1] = part1_world;
            Quat rot = part1_world.GetQuaternion();
            write_cframe(model, *joint.part1, part1_world.GetTranslation(), rot);
        }
        return;
    }
    auto place_at = [&](const char* name, double right_off, double y, double swing_sign) {
        for (Instance* part : ch.parts) {
            if (part->name != name) continue;
            Vec3 pos = centre + right * static_cast<float>(right_off) + Vec3(0, static_cast<float>(y), 0) +
                       back * static_cast<float>(swing * swing_sign * 0.3);
            Quat rot = facing;
            if (swing_sign != 0.0)
                rot = facing * Quat::sRotation(Vec3::sAxisX(), static_cast<float>(swing * swing_sign * 0.7));
            write_cframe(model, *part, pos, rot);
            return;
        }
    };
    place_at("HumanoidRootPart", 0, feet_y + 2.0 - char_pos.GetY(), 0);
    place_at("Torso", 0, feet_y + 2.9 - char_pos.GetY(), 0);
    place_at("LowerTorso", 0, feet_y + 2.4 - char_pos.GetY(), 0);
    place_at("UpperTorso", 0, feet_y + 3.3 - char_pos.GetY(), 0);
    place_at("Head", 0, feet_y + 4.4 - char_pos.GetY(), 0);
    place_at("Left Arm", 1.5, feet_y + 2.9 - char_pos.GetY(), 1);
    place_at("Right Arm", -1.5, feet_y + 2.9 - char_pos.GetY(), -1);
    place_at("LeftUpperArm", 1.4, feet_y + 3.2 - char_pos.GetY(), 1);
    place_at("RightUpperArm", -1.4, feet_y + 3.2 - char_pos.GetY(), -1);
    place_at("LeftLowerArm", 1.4, feet_y + 2.4 - char_pos.GetY(), 1);
    place_at("RightLowerArm", -1.4, feet_y + 2.4 - char_pos.GetY(), -1);
    place_at("LeftHand", 1.4, feet_y + 1.8 - char_pos.GetY(), 1);
    place_at("RightHand", -1.4, feet_y + 1.8 - char_pos.GetY(), -1);
    place_at("Left Leg", 0.5, feet_y + 0.9 - char_pos.GetY(), -1);
    place_at("Right Leg", -0.5, feet_y + 0.9 - char_pos.GetY(), 1);
    place_at("LeftUpperLeg", 0.5, feet_y + 1.5 - char_pos.GetY(), -1);
    place_at("RightUpperLeg", -0.5, feet_y + 1.5 - char_pos.GetY(), 1);
    place_at("LeftLowerLeg", 0.5, feet_y + 0.7 - char_pos.GetY(), -1);
    place_at("RightLowerLeg", -0.5, feet_y + 0.7 - char_pos.GetY(), 1);
    place_at("LeftFoot", 0.5, feet_y + 0.2 - char_pos.GetY(), -1);
    place_at("RightFoot", -0.5, feet_y + 0.2 - char_pos.GetY(), 1);
}

std::vector<Instance*> PhysicsWorld::step(double dt) {
    Impl& im = *impl_;
    std::vector<Instance*> moved;
    // --- character updates ---
    for (Impl::CharState& ch : im.chars) {
        double walk_speed = ch.humanoid ? prop_number(*ch.humanoid, "WalkSpeed", 16.0) : 16.0;
        double jump_power = ch.humanoid ? prop_number(*ch.humanoid, "JumpPower", 50.0) : 50.0;
        CharacterBase::EGroundState ground = ch.character->GetGroundState();
        bool grounded = ground == CharacterBase::EGroundState::OnGround ||
                        ground == CharacterBase::EGroundState::OnSteepGround;
        double sy = std::sin(ch.input_yaw), cyaw = std::cos(ch.input_yaw);
        std::array<double, 2> wish{(-sy) * ch.move_input[1] + cyaw * ch.move_input[0],
                                   (-cyaw) * ch.move_input[1] + (-sy) * ch.move_input[0]};
        double wish_len = std::sqrt(wish[0] * wish[0] + wish[1] * wish[1]);
        if (wish_len > 1.0) { wish[0] /= wish_len; wish[1] /= wish_len; wish_len = 1.0; }
        if (wish_len > 0.05) {
            ch.facing_yaw = std::atan2(-wish[0], -wish[1]);
        }
        ch.prev_vertical_velocity = ch.vertical_velocity;
        if (grounded) {
            ch.vertical_velocity = ch.jump_input ? jump_power : -1.0;
        } else {
            ch.vertical_velocity = std::max(-250.0, ch.vertical_velocity - im.gravity * dt);
        }
        Vec3 velocity(static_cast<float>(wish[0] * walk_speed),
                      static_cast<float>(ch.vertical_velocity),
                      static_cast<float>(wish[1] * walk_speed));
        ch.character->SetLinearVelocity(velocity);
        im.update_settings.mStickToFloorStepDown = Vec3(0, -1.0f, 0);
        im.update_settings.mWalkStairsStepUp = Vec3(0, 1.0f, 0);
        ch.character->ExtendedUpdate(static_cast<float>(dt),
                                     Vec3(0, static_cast<float>(-im.gravity), 0),
                                     im.update_settings, im.bp_layer_filter,
                                     im.object_layer_filter, im.body_filter, im.shape_filter,
                                     im.allocator);
        bool now_grounded = ch.character->GetGroundState() == CharacterBase::EGroundState::OnGround;
        if (now_grounded && !grounded && ch.prev_vertical_velocity < -75.0 && ch.humanoid) {
            double damage = std::min(100.0, (-ch.prev_vertical_velocity - 75.0) * 0.8);
            double health = prop_number(*ch.humanoid, "Health", 100.0) - damage;
            std::string ignored;
            model_.set_property(*ch.humanoid, "Health", Variant::make_float64(health), ignored);
            if (health <= 0) ch.events.push_back("died");
        }
        if (ch.humanoid && prop_number(*ch.humanoid, "Health", 100.0) <= 0 &&
            std::find(ch.events.begin(), ch.events.end(), "died") == ch.events.end()) {
            ch.events.push_back("died");
        }
        Vec3 pos = ch.character->GetPosition();
        if (pos.GetY() < -100.0) {
            ch.character->SetPosition(Vec3(static_cast<float>(im.spawn_point[0]),
                                           static_cast<float>(im.spawn_point[1]),
                                           static_cast<float>(im.spawn_point[2])));
            ch.vertical_velocity = 0;
            if (ch.humanoid) {
                std::string ignored;
                model_.set_property(*ch.humanoid, "Health", Variant::make_float64(100.0), ignored);
            }
            ch.events.push_back("respawned");
            pos = ch.character->GetPosition();
        }
        pose_character(im, model_, ch, pos, dt);
        for (Instance* part : ch.parts) moved.push_back(part);
    }
    // --- world step ---
    im.physics.Update(static_cast<float>(dt), 1, &im.allocator, &im.jobs);
    time_ += dt;
    {
        // Touch-begin detection: AABB overlap transitions involving at
        // least one dynamic body (this is overlap-based, not a Jolt
        // contact listener; see COMPATIBILITY.md).
        BodyInterface& bi = im.physics.GetBodyInterfaceNoLock();
        std::set<std::pair<BodyID, BodyID>> now_touching;
        std::vector<std::pair<BodyID, AABox>> bounds;
        for (const auto& [inst, id] : im.bodies) {
            BodyLockRead lock(im.physics.GetBodyLockInterface(), id);
            bounds.push_back({id, lock.GetBody().GetWorldSpaceBounds()});
            (void)inst;
        }
        for (size_t i = 0; i < bounds.size(); ++i) {
            bool dyn_i = bi.GetMotionType(bounds[i].first) == EMotionType::Dynamic;
            for (size_t j = i + 1; j < bounds.size(); ++j) {
                bool dyn_j = bi.GetMotionType(bounds[j].first) == EMotionType::Dynamic;
                if (!dyn_i && !dyn_j) continue;
                if (!bounds[i].second.Overlaps(bounds[j].second)) continue;
                auto key = std::minmax(bounds[i].first, bounds[j].first);
                now_touching.insert(key);
                if (!im.touching.contains(key)) {
                    Instance* a = im.by_id[bounds[i].first];
                    Instance* b = im.by_id[bounds[j].first];
                    // Exclude two parts of the same character rig (they
                    // have no bodies, so this is just belt-and-braces).
                    im.touch_events.push_back({a, b});
                    im.touch_events.push_back({b, a});
                }
            }
        }
        im.touching = std::move(now_touching);
    }
    BodyInterface& bodies = im.physics.GetBodyInterfaceNoLock();
    for (auto& [inst, id] : im.bodies) {
        if (bodies.GetMotionType(id) == EMotionType::Static) continue;
        Vec3 pos;
        Quat rot;
        bodies.GetPositionAndRotation(id, pos, rot);
        Variant before = cframe_of(*inst);
        double dx = pos.GetX() - before.position[0];
        double dy = pos.GetY() - before.position[1];
        double dz = pos.GetZ() - before.position[2];
        if (dx * dx + dy * dy + dz * dz > 1e-8) {
            write_cframe(model_, *inst, pos, rot);
            moved.push_back(inst);
        }
    }
    return moved;
}


namespace {
// Ray collector honoring an instance allowlist (Roblox raycast filters).
class AllowlistRayCollector final : public CastRayCollector {
public:
    AllowlistRayCollector(const PhysicsSystem& system,
                          const std::map<BodyID, Instance*>& by_id,
                          const std::unordered_set<Instance*>* allow)
        : system_(system), by_id_(by_id), allow_(allow) {}
    void Reset() override {
        CastRayCollector::Reset();
        m_hit = RayCastResult();
    }
    void AddHit(const RayCastResult& result) override {
        auto it = by_id_.find(result.mBodyID);
        if (it == by_id_.end()) return;
        if (allow_ && !allow_->contains(it->second)) return;
        if (result.mFraction < m_hit.mFraction) {
            m_hit = result;
            UpdateEarlyOutFraction(m_hit.mFraction);
        }
    }
    RayCastResult m_hit;
private:
    const PhysicsSystem& system_;
    const std::map<BodyID, Instance*>& by_id_;
    const std::unordered_set<Instance*>* allow_;
};
} // namespace

PhysicsWorld::RayHit PhysicsWorld::raycast_filtered(std::array<double, 3> origin,
                                                    std::array<double, 3> direction,
                                                    double max_distance,
                                                    const std::vector<Instance*>* allow_only) const {
    RayHit out;
    double len = std::sqrt(direction[0] * direction[0] + direction[1] * direction[1] +
                           direction[2] * direction[2]);
    if (len < 1e-9) return out;
    Vec3 dir(static_cast<float>(direction[0] / len * max_distance),
             static_cast<float>(direction[1] / len * max_distance),
             static_cast<float>(direction[2] / len * max_distance));
    RRayCast ray(Vec3(static_cast<float>(origin[0]), static_cast<float>(origin[1]),
                      static_cast<float>(origin[2])), dir);
    std::unordered_set<Instance*> allow_set;
    const std::unordered_set<Instance*>* allow_ptr = nullptr;
    if (allow_only) {
        allow_set.insert(allow_only->begin(), allow_only->end());
        allow_ptr = &allow_set;
    }
    AllowlistRayCollector collector(impl_->physics, impl_->by_id, allow_ptr);
    RayCastSettings settings;
    impl_->physics.GetNarrowPhaseQuery().CastRay(ray, settings, collector);
    const RayCastResult& result = collector.m_hit;
    if (result.mFraction > 1.0f) return out;
    auto it = impl_->by_id.find(result.mBodyID);
    if (it == impl_->by_id.end()) return out;
    out.hit = true;
    out.instance = it->second;
    out.path = model_.path_of(*it->second);
    Vec3 point = ray.GetPointOnRay(result.mFraction);
    out.point = {point.GetX(), point.GetY(), point.GetZ()};
    out.distance = result.mFraction * max_distance;
    return out;
}

PhysicsWorld::RayHit PhysicsWorld::raycast(std::array<double, 3> origin,
                                           std::array<double, 3> direction,
                                           double max_distance) const {
    RayHit out;
    double len = std::sqrt(direction[0] * direction[0] + direction[1] * direction[1] +
                           direction[2] * direction[2]);
    if (len < 1e-9) return out;
    Vec3 dir(static_cast<float>(direction[0] / len * max_distance),
             static_cast<float>(direction[1] / len * max_distance),
             static_cast<float>(direction[2] / len * max_distance));
    RRayCast ray(Vec3(static_cast<float>(origin[0]), static_cast<float>(origin[1]),
                      static_cast<float>(origin[2])), dir);
    RayCastResult result;
    impl_->physics.GetNarrowPhaseQuery().CastRay(ray, result);
    if (result.mFraction > 1.0f) return out;
    auto it = impl_->by_id.find(result.mBodyID);
    if (it == impl_->by_id.end()) return out;
    out.hit = true;
    out.instance = it->second;
    out.path = model_.path_of(*it->second);
    Vec3 point = ray.GetPointOnRay(result.mFraction);
    out.point = {point.GetX(), point.GetY(), point.GetZ()};
    out.distance = result.mFraction * max_distance;
    return out;
}

} // namespace blox