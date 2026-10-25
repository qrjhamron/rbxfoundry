#include "blox/datamodel.hpp"
#include "blox/brickcolor.hpp"

#include <algorithm>
#include <cmath>
#include <functional>
#include <set>

namespace blox {
namespace {

Variant color(double r, double g, double b) { return Variant::make_color3(r, g, b); }

Variant default_color_sequence() {
    Variant v;
    v.type = ValueType::ColorSequence;
    Json color = Json::array();
    color.push(Json(1.0));
    color.push(Json(1.0));
    color.push(Json(1.0));
    Json kp = Json::object();
    kp.set("time", Json(0.0));
    kp.set("color", std::move(color));
    Json kp2 = Json::object();
    kp2.set("time", Json(1.0));
    Json color2 = Json::array();
    color2.push(Json(1.0));
    color2.push(Json(1.0));
    color2.push(Json(1.0));
    kp2.set("color", std::move(color2));
    v.complex_value = Json::array();
    v.complex_value.push(std::move(kp));
    v.complex_value.push(std::move(kp2));
    return v;
}
Variant default_number_sequence(double value) {
    Variant v;
    v.type = ValueType::NumberSequence;
    Json kp = Json::object();
    kp.set("time", Json(0.0));
    kp.set("value", Json(value));
    kp.set("envelope", Json(0.0));
    Json kp2 = Json::object();
    kp2.set("time", Json(1.0));
    kp2.set("value", Json(value));
    kp2.set("envelope", Json(0.0));
    v.complex_value = Json::array();
    v.complex_value.push(std::move(kp));
    v.complex_value.push(std::move(kp2));
    return v;
}
Variant default_number_range(double lo, double hi) {
    Variant v;
    v.type = ValueType::NumberRange;
    v.complex_value = Json::array();
    v.complex_value.push(Json(lo));
    v.complex_value.push(Json(hi));
    return v;
}

const std::map<std::string, std::map<std::string, PropertyInfo>>& reflection_table() {
    using V = ValueType;
    static const std::map<std::string, std::map<std::string, PropertyInfo>> table = {
        {"Workspace", {
            {"Gravity", {V::Float64, Variant::make_float64(196.2)}},
            {"StreamingEnabled", {V::Bool, Variant::make_bool(false)}},
        }},
        {"Lighting", {
            {"ClockTime", {V::Float64, Variant::make_float64(14.5)}},
            {"Brightness", {V::Float64, Variant::make_float64(2.0)}},
            {"Ambient", {V::Color3, color(0, 0, 0)}},
            {"OutdoorAmbient", {V::Color3, color(0.5, 0.5, 0.5)}},
            {"GlobalShadows", {V::Bool, Variant::make_bool(true)}},
            {"GeographicLatitude", {V::Float64, Variant::make_float64(41.733)}},
            {"ExposureCompensation", {V::Float64, Variant::make_float64(0.0)}},
            {"EnvironmentDiffuseScale", {V::Float64, Variant::make_float64(0.0)}},
            {"EnvironmentSpecularScale", {V::Float64, Variant::make_float64(0.0)}},
        }},
        {"StarterPlayer", {
            {"CharacterWalkSpeed", {V::Float64, Variant::make_float64(16.0)}},
            {"CharacterJumpPower", {V::Float64, Variant::make_float64(50.0)}},
            {"CharacterJumpHeight", {V::Float64, Variant::make_float64(7.2)}},
            {"CameraMaxZoomDistance", {V::Float64, Variant::make_float64(400.0)}},
            {"CameraMinZoomDistance", {V::Float64, Variant::make_float64(0.5)}},
            {"CameraMode", {V::Enum, Variant::make_enum(0)}},
            {"EnableMouseLockOption", {V::Bool, Variant::make_bool(true)}},
        }},
        {"Part", {
            {"Anchored", {V::Bool, Variant::make_bool(false)}},
            {"CanCollide", {V::Bool, Variant::make_bool(true)}},
            {"CanQuery", {V::Bool, Variant::make_bool(true)}},
            {"CanTouch", {V::Bool, Variant::make_bool(true)}},
            {"CastShadow", {V::Bool, Variant::make_bool(true)}},
            {"Locked", {V::Bool, Variant::make_bool(false)}},
            {"Massless", {V::Bool, Variant::make_bool(false)}},
            {"RootPriority", {V::Int32, Variant::make_int64(0)}},
            {"Transparency", {V::Float64, Variant::make_float64(0.0)}},
            {"Reflectance", {V::Float64, Variant::make_float64(0.0)}},
            {"Color", {V::Color3, color(0.639216, 0.635294, 0.647059)}},
            {"BrickColor", {V::BrickColor, [] { Variant v; v.type = V::BrickColor; v.int_value = 194; return v; }()}},
            {"Material", {V::Enum, Variant::make_enum(256)}},
            {"MaterialVariant", {V::String, Variant::make_string("")}},
            {"Shape", {V::Enum, Variant::make_enum(1)}},
            {"Size", {V::Vector3, Variant::make_vec3(4, 1, 2)}},
            {"CFrame", {V::CFrame, Variant::make_cframe(0, 0, 0)}},
            {"CustomPhysicalProperties", {V::PhysicalProperties, [] { Variant v; v.type = V::PhysicalProperties; v.complex_value = Json("Default"); return v; }()}},
        }},
        {"SpawnLocation", {
            {"Anchored", {V::Bool, Variant::make_bool(true)}},
            {"CanCollide", {V::Bool, Variant::make_bool(true)}},
            {"Transparency", {V::Float64, Variant::make_float64(0.0)}},
            {"Color", {V::Color3, color(0.639216, 0.635294, 0.647059)}},
            {"Material", {V::Enum, Variant::make_enum(256)}},
            {"Size", {V::Vector3, Variant::make_vec3(6, 1, 6)}},
            {"CFrame", {V::CFrame, Variant::make_cframe(0, 0, 0)}},
            {"Enabled", {V::Bool, Variant::make_bool(true)}},
            {"Neutral", {V::Bool, Variant::make_bool(true)}},
            {"Duration", {V::Int32, Variant::make_int64(10)}},
        }},
        {"Script", {
            {"Source", {V::String, Variant::make_string("")}},
            {"Disabled", {V::Bool, Variant::make_bool(false)}},
        }},
        {"LocalScript", {
            {"Source", {V::String, Variant::make_string("")}},
            {"Disabled", {V::Bool, Variant::make_bool(false)}},
        }},
        {"ModuleScript", {
            {"Source", {V::String, Variant::make_string("")}},
        }},
        {"Model", {{"WorldPivot", {V::CFrame, Variant::make_cframe(0, 0, 0)}}}},
        {"Terrain", {
            {"BloxVoxels", {V::BinaryString, [] { Variant v; v.type = V::BinaryString; return v; }()}},
            {"WaterColor", {V::Color3, color(0.11, 0.35, 0.64)}},
            {"WaterTransparency", {V::Float64, Variant::make_float64(0.3)}},
            {"WaterWaveSize", {V::Float64, Variant::make_float64(0.15)}},
            {"WaterWaveSpeed", {V::Float64, Variant::make_float64(10.0)}},
        }},
        {"Camera", {
            {"CFrame", {V::CFrame, Variant::make_cframe(0, 0, 0)}},
            {"FieldOfView", {V::Float64, Variant::make_float64(70.0)}},
        }},
        {"Decal", {
            {"Texture", {V::ContentId, [] { Variant v; v.type = V::ContentId; return v; }()}},
            {"Transparency", {V::Float64, Variant::make_float64(0.0)}},
            {"Color3", {V::Color3, color(1, 1, 1)}},
            {"ZIndex", {V::Int32, Variant::make_int64(1)}},
            {"Face", {V::Enum, Variant::make_enum(5)}},
        }},
        {"Texture", {
            {"Texture", {V::ContentId, [] { Variant v; v.type = V::ContentId; return v; }()}},
            {"Transparency", {V::Float64, Variant::make_float64(0.0)}},
            {"Color3", {V::Color3, color(1, 1, 1)}},
            {"ZIndex", {V::Int32, Variant::make_int64(1)}},
            {"Face", {V::Enum, Variant::make_enum(5)}},
            {"StudsPerTileU", {V::Float64, Variant::make_float64(2.0)}},
            {"StudsPerTileV", {V::Float64, Variant::make_float64(2.0)}},
        }},
        {"StringValue", {{"Value", {V::String, Variant::make_string("")}}}},
        {"IntValue", {{"Value", {V::Int64, Variant::make_int64(0)}}}},
        {"NumberValue", {{"Value", {V::Float64, Variant::make_float64(0.0)}}}},
        {"BoolValue", {{"Value", {V::Bool, Variant::make_bool(false)}}}},
        {"Color3Value", {{"Value", {V::Color3, color(0, 0, 0)}}}},
        {"Sound", {
            {"SoundId", {V::ContentId, [] { Variant v; v.type = V::ContentId; return v; }()}},
            {"Volume", {V::Float64, Variant::make_float64(0.5)}},
            {"Playing", {V::Bool, Variant::make_bool(false)}},
        }},
        {"PointLight", {
            {"Brightness", {V::Float64, Variant::make_float64(1.0)}},
            {"Range", {V::Float64, Variant::make_float64(8.0)}},
            {"Color", {V::Color3, color(1, 1, 1)}},
            {"Shadows", {V::Bool, Variant::make_bool(false)}},
        }},
        {"SpotLight", {
            {"Brightness", {V::Float64, Variant::make_float64(1.0)}},
            {"Range", {V::Float64, Variant::make_float64(16.0)}},
            {"Color", {V::Color3, color(1, 1, 1)}},
            {"Angle", {V::Float64, Variant::make_float64(90.0)}},
            {"Face", {V::Enum, Variant::make_enum(5)}},
            {"Shadows", {V::Bool, Variant::make_bool(false)}},
        }},
        {"SurfaceLight", {
            {"Brightness", {V::Float64, Variant::make_float64(1.0)}},
            {"Range", {V::Float64, Variant::make_float64(16.0)}},
            {"Color", {V::Color3, color(1, 1, 1)}},
            {"Angle", {V::Float64, Variant::make_float64(90.0)}},
            {"Face", {V::Enum, Variant::make_enum(5)}},
            {"Shadows", {V::Bool, Variant::make_bool(false)}},
        }},
        {"SurfaceAppearance", {
            {"ColorMap", {V::ContentId, [] { Variant v; v.type = V::ContentId; return v; }()}},
            {"NormalMap", {V::ContentId, [] { Variant v; v.type = V::ContentId; return v; }()}},
            {"RoughnessMap", {V::ContentId, [] { Variant v; v.type = V::ContentId; return v; }()}},
            {"MetalnessMap", {V::ContentId, [] { Variant v; v.type = V::ContentId; return v; }()}},
            {"AlphaMode", {V::Enum, Variant::make_enum(0)}},
        }},
        {"Atmosphere", {
            {"Density", {V::Float64, Variant::make_float64(0.0)}},
            {"Offset", {V::Float64, Variant::make_float64(0.0)}},
            {"Glare", {V::Float64, Variant::make_float64(0.0)}},
            {"Haze", {V::Float64, Variant::make_float64(0.0)}},
            {"Color", {V::Color3, color(199.0 / 255.0, 215.0 / 255.0, 235.0 / 255.0)}},
            {"Decay", {V::Color3, color(110.0 / 255.0, 125.0 / 255.0, 145.0 / 255.0)}},
        }},
        {"Beam", {
            {"Attachment0", {V::Ref, [] { Variant v; v.type = V::Ref; return v; }()}},
            {"Attachment1", {V::Ref, [] { Variant v; v.type = V::Ref; return v; }()}},
            {"Width0", {V::Float64, Variant::make_float64(1.0)}},
            {"Width1", {V::Float64, Variant::make_float64(1.0)}},
            {"Transparency", {V::NumberSequence, default_number_sequence(0.0)}},
            {"Color", {V::ColorSequence, default_color_sequence()}},
        }},
        {"Trail", {
            {"Attachment0", {V::Ref, [] { Variant v; v.type = V::Ref; return v; }()}},
            {"Attachment1", {V::Ref, [] { Variant v; v.type = V::Ref; return v; }()}},
            {"WidthScale", {V::NumberSequence, default_number_sequence(1.0)}},
            {"Transparency", {V::NumberSequence, default_number_sequence(0.0)}},
            {"Color", {V::ColorSequence, default_color_sequence()}},
        }},
        {"ParticleEmitter", {
            {"Rate", {V::Float64, Variant::make_float64(20.0)}},
            {"Lifetime", {V::NumberRange, default_number_range(5.0, 5.0)}},
            {"Speed", {V::NumberRange, default_number_range(5.0, 5.0)}},
            {"Size", {V::NumberSequence, default_number_sequence(1.0)}},
            {"Transparency", {V::NumberSequence, default_number_sequence(0.0)}},
            {"Color", {V::ColorSequence, default_color_sequence()}},
            {"EmissionDirection", {V::Enum, Variant::make_enum(1)}},
            {"Enabled", {V::Bool, Variant::make_bool(true)}},
        }},
        {"Motor6D", {
            {"Part0", {V::Ref, [] { Variant v; v.type = V::Ref; return v; }()}},
            {"Part1", {V::Ref, [] { Variant v; v.type = V::Ref; return v; }()}},
            {"C0", {V::CFrame, Variant::make_cframe(0, 0, 0)}},
            {"C1", {V::CFrame, Variant::make_cframe(0, 0, 0)}},
            {"Transform", {V::CFrame, Variant::make_cframe(0, 0, 0)}},
            {"MaxVelocity", {V::Float64, Variant::make_float64(0.1)}},
        }},
        {"Weld", {
            {"Part0", {V::Ref, [] { Variant v; v.type = V::Ref; return v; }()}},
            {"Part1", {V::Ref, [] { Variant v; v.type = V::Ref; return v; }()}},
            {"C0", {V::CFrame, Variant::make_cframe(0, 0, 0)}},
            {"C1", {V::CFrame, Variant::make_cframe(0, 0, 0)}},
        }},
        {"WeldConstraint", {
            {"Part0", {V::Ref, [] { Variant v; v.type = V::Ref; return v; }()}},
            {"Part1", {V::Ref, [] { Variant v; v.type = V::Ref; return v; }()}},
        }},
        {"BallSocketConstraint", {
            {"Attachment0", {V::Ref, [] { Variant v; v.type = V::Ref; return v; }()}},
            {"Attachment1", {V::Ref, [] { Variant v; v.type = V::Ref; return v; }()}},
        }},
        {"HingeConstraint", {
            {"Attachment0", {V::Ref, [] { Variant v; v.type = V::Ref; return v; }()}},
            {"Attachment1", {V::Ref, [] { Variant v; v.type = V::Ref; return v; }()}},
        }},
        {"Attachment", {
            {"CFrame", {V::CFrame, Variant::make_cframe(0, 0, 0)}},
            {"Axis", {V::Vector3, Variant::make_vec3(1, 0, 0)}},
            {"SecondaryAxis", {V::Vector3, Variant::make_vec3(0, 1, 0)}},
        }},
        {"Animation", {
            {"AnimationId", {V::ContentId, [] { Variant v; v.type = V::ContentId; return v; }()}},
        }},
        {"KeyframeSequence", {
            {"Loop", {V::Bool, Variant::make_bool(false)}},
            {"Priority", {V::Enum, Variant::make_enum(0)}},
        }},
        {"Keyframe", {
            {"Time", {V::Float64, Variant::make_float64(0.0)}},
        }},
        {"Pose", {
            {"CFrame", {V::CFrame, Variant::make_cframe(0, 0, 0)}},
            {"Weight", {V::Float64, Variant::make_float64(1.0)}},
        }},
        {"DataModel", {
            {"PlaceId", {V::Int64, Variant::make_int64(0)}},
            {"GameId", {V::String, Variant::make_string("")}},
            {"PlaceVersion", {V::Int64, Variant::make_int64(0)}},
            {"JobId", {V::String, Variant::make_string("")}},
            {"CreatorId", {V::Int64, Variant::make_int64(0)}},
            {"CreatorType", {V::Enum, Variant::make_enum(0)}},
        }},
        {"SpawnLocation", {
            {"Neutral", {V::Bool, Variant::make_bool(true)}},
            {"AllowTeamChangeOnTouch", {V::Bool, Variant::make_bool(true)}},
            {"Duration", {V::Float64, Variant::make_float64(0.0)}},
            {"ForceFieldDuration", {V::Float64, Variant::make_float64(10.0)}},
        }},
        {"Camera", {
            {"CameraType", {V::Enum, Variant::make_enum(0)}},
            {"FieldOfView", {V::Float64, Variant::make_float64(70.0)}},
            {"CFrame", {V::CFrame, Variant::make_cframe(0, 0, 0)}},
            {"Focus", {V::CFrame, Variant::make_cframe(0, 0, 0)}},
        }},
        {"Humanoid", {
            {"Health", {V::Float64, Variant::make_float64(100.0)}},
            {"MaxHealth", {V::Float64, Variant::make_float64(100.0)}},
            {"WalkSpeed", {V::Float64, Variant::make_float64(16.0)}},
            {"JumpPower", {V::Float64, Variant::make_float64(50.0)}},
        }},
    };
    return table;
}

const std::set<std::string>& known_classes() {
    static const std::set<std::string> classes = {
        "DataModel", "Workspace", "Lighting", "ReplicatedStorage", "ServerStorage",
        "ServerScriptService", "StarterGui", "StarterPack", "StarterPlayer",
        "StarterCharacterScripts", "Teams", "Team", "SoundService", "Players", "Player",
        "Folder", "Model", "Terrain", "Part", "MeshPart", "SpawnLocation", "Seat", "WedgePart",
        "CornerWedgePart", "UnionOperation", "NegateOperation", "Script", "LocalScript",
        "ModuleScript", "Camera", "StringValue", "IntValue", "NumberValue", "BoolValue",
        "Color3Value", "ObjectValue", "Sound", "PointLight", "SpotLight", "SurfaceLight",
        "Decal", "Texture", "Sky", "Humanoid", "Accessory", "Tool", "ScreenGui", "Frame",
        "TextLabel", "TextButton", "ImageLabel", "BillboardGui", "SurfaceGui",
        "ParticleEmitter", "Beam", "Trail", "Fire", "Smoke", "Sparkles", "Atmosphere", "BloomEffect",
        "RunService", "TweenService", "CollectionService", "UserInputService", "TextService", "PhysicsService", "ContextActionService", "PathfindingService", "TextBox",
        "ImageButton", "ScrollingFrame", "CanvasGroup", "UICorner", "UIStroke", "UIGradient",
        "UIPadding", "UIScale", "UIListLayout", "UIGridLayout", "UIPageLayout",
        "UIAspectRatioConstraint", "UISizeConstraint", "UITextSizeConstraint", "SurfaceAppearance",
        "ReplicatedFirst", "StarterPlayerScripts", "BindableEvent", "BindableFunction",
        "RemoteEvent", "RemoteFunction", "Motor6D", "Weld", "WeldConstraint", "Attachment",
        "BallSocketConstraint", "HingeConstraint",
        "Animation", "Animator", "KeyframeSequence", "Keyframe", "Pose", "Shirt", "Pants", "BodyColors",
        "BlurEffect", "ColorCorrectionEffect", "SunRaysEffect", "DepthOfFieldEffect",
    };
    return classes;
}

// Classes that inherit Part-like defaults without their own table entry.
const std::map<std::string, PropertyInfo>* part_like(const std::string& class_name) {
    static const std::set<std::string> part_like_classes = {
        "MeshPart", "Seat", "WedgePart", "CornerWedgePart", "UnionOperation", "NegateOperation"};
    if (part_like_classes.count(class_name)) return reflection_for("Part");
    return nullptr;
}

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

} // namespace


const std::map<std::string, PropertyInfo>* reflection_for(const std::string& class_name) {
    const auto& table = reflection_table();
    auto it = table.find(class_name);
    if (it != table.end()) return &it->second;
    if (class_name == "MeshPart") {
        // BasePart property set plus mesh-specific properties.
        static const std::map<std::string, PropertyInfo> mesh_part = [] {
            auto merged = reflection_table().at("Part");
            Variant mesh_id;
            mesh_id.type = ValueType::ContentId;
            merged["MeshId"] = {ValueType::ContentId, mesh_id};
            merged["TextureID"] = {ValueType::ContentId, mesh_id};
            merged["DoubleSided"] = {ValueType::Bool, Variant::make_bool(false)};
            merged["RenderFidelity"] = {ValueType::Enum, Variant::make_enum(0)};
            merged.erase("Shape"); // Part-only; MeshPart derives from BasePart
            return merged;
        }();
        return &mesh_part;
    }
    return part_like(class_name);
}

bool is_known_class(const std::string& class_name) { return known_classes().count(class_name) > 0; }

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
    auto parts = split_path(path);
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
            // Allow numeric widening between Int32/Int64 and Float32/Float64 pairs.
            bool int_pair = (it->second.type == ValueType::Int32 || it->second.type == ValueType::Int64) &&
                            (value.type == ValueType::Int32 || value.type == ValueType::Int64);
            bool float_pair = (it->second.type == ValueType::Float32 || it->second.type == ValueType::Float64) &&
                              (value.type == ValueType::Float32 || value.type == ValueType::Float64);
            if (!int_pair && !float_pair) {
                error = "property '" + property + "' on " + inst.class_name + " expects " +
                        value_type_name(it->second.type) + ", got " + value_type_name(value.type);
                return false;
            }
            Variant coerced = value;
            coerced.type = it->second.type;
            inst.properties[property] = coerced;
            if (property == "CFrame") sync_position_from_cframe(inst);
            sync_color_brick(inst, property);
            return true;
        }
    }
    inst.properties[property] = value;
    if (property == "CFrame") sync_position_from_cframe(inst);
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
    sync_position_from_cframe(inst);
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
