// Reflection metadata: default properties per class and the known-class
// registry. DataModel::apply_defaults and property type checking consume
// this table; unknown classes/properties are preserved verbatim.
#include "blox/datamodel/datamodel.hpp"
#include "blox/core/brickcolor.hpp"

#include <map>
#include <set>
#include <string>

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


Variant udim2(double xs, double xo, double ys, double yo) {
    Variant v; v.type = ValueType::UDim2;
    v.complex_value = Json::array();
    v.complex_value.push(Json(xs)); v.complex_value.push(Json(xo));
    v.complex_value.push(Json(ys)); v.complex_value.push(Json(yo));
    return v;
}
Variant vec2(double x, double y) {
    Variant v; v.type = ValueType::Vector2;
    v.complex_value = Json::array();
    v.complex_value.push(Json(x)); v.complex_value.push(Json(y));
    return v;
}
Variant f32(double x) { Variant v = Variant::make_float64(x); v.type = ValueType::Float32; return v; }
Variant i32(int64_t x) { Variant v = Variant::make_int64(x); v.type = ValueType::Int32; return v; }
Variant content(const std::string& s) { Variant v; v.type = ValueType::ContentId; v.string_value = s; return v; }

// GuiObject property set shared by Frame/Text*/Image*/ScrollingFrame
// (Gate A2/27C). Types follow Roblox reflection; defaults are the
// documented creation defaults for the covered properties only.
std::map<std::string, PropertyInfo> gui_object_props() {
    return {
        {"Active", {ValueType::Bool, Variant::make_bool(false)}},
        {"AnchorPoint", {ValueType::Vector2, vec2(0, 0)}},
        {"AutomaticSize", {ValueType::Enum, Variant::make_enum(0)}},
        {"BackgroundColor3", {ValueType::Color3, color(1, 1, 1)}},
        {"BackgroundTransparency", {ValueType::Float32, f32(0)}},
        {"BorderColor3", {ValueType::Color3, color(0, 0, 0)}},
        {"BorderMode", {ValueType::Enum, Variant::make_enum(0)}},
        {"BorderSizePixel", {ValueType::Int32, i32(1)}},
        {"ClipsDescendants", {ValueType::Bool, Variant::make_bool(false)}},
        {"LayoutOrder", {ValueType::Int32, i32(0)}},
        {"Position", {ValueType::UDim2, udim2(0, 0, 0, 0)}},
        {"Rotation", {ValueType::Float32, f32(0)}},
        {"Size", {ValueType::UDim2, udim2(0, 0, 0, 0)}},
        {"SizeConstraint", {ValueType::Enum, Variant::make_enum(0)}},
        {"Visible", {ValueType::Bool, Variant::make_bool(true)}},
        {"ZIndex", {ValueType::Int32, i32(1)}},
        {"Selectable", {ValueType::Bool, Variant::make_bool(false)}},
        {"SelectionOrder", {ValueType::Int32, i32(0)}},
    };
}
std::map<std::string, PropertyInfo> text_props() {
    auto props = gui_object_props();
    props["Font"] = {ValueType::Enum, Variant::make_enum(3)};
    props["Text"] = {ValueType::String, Variant::make_string("")};
    props["TextColor3"] = {ValueType::Color3, color(0, 0, 0)};
    props["TextSize"] = {ValueType::Float32, f32(14)};
    props["TextTransparency"] = {ValueType::Float32, f32(0)};
    props["TextStrokeColor3"] = {ValueType::Color3, color(0, 0, 0)};
    props["TextStrokeTransparency"] = {ValueType::Float32, f32(1)};
    props["TextWrapped"] = {ValueType::Bool, Variant::make_bool(false)};
    props["TextScaled"] = {ValueType::Bool, Variant::make_bool(false)};
    props["TextXAlignment"] = {ValueType::Enum, Variant::make_enum(0)};
    props["TextYAlignment"] = {ValueType::Enum, Variant::make_enum(1)};
    props["RichText"] = {ValueType::Bool, Variant::make_bool(false)};
    props["LineHeight"] = {ValueType::Float32, f32(1)};
    return props;
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
    // GUI classes (Gate A2/27C): typed defaults so Luau/CLI assignments
    // are reflection-checked and serialize with correct types.
    {
        static const std::map<std::string, std::map<std::string, PropertyInfo>> gui = [] {
            std::map<std::string, std::map<std::string, PropertyInfo>> m;
            m["Frame"] = gui_object_props();
            m["CanvasGroup"] = gui_object_props();
            m["CanvasGroup"]["GroupColor3"] = {ValueType::Color3, color(1, 1, 1)};
            m["CanvasGroup"]["GroupTransparency"] = {ValueType::Float32, f32(0)};
            m["TextLabel"] = text_props();
            m["TextButton"] = text_props();
            m["TextButton"]["AutoButtonColor"] = {ValueType::Bool, Variant::make_bool(true)};
            m["TextButton"]["Modal"] = {ValueType::Bool, Variant::make_bool(false)};
            m["TextBox"] = text_props();
            m["TextBox"]["PlaceholderText"] = {ValueType::String, Variant::make_string("")};
            m["TextBox"]["PlaceholderColor3"] = {ValueType::Color3, color(0.7, 0.7, 0.7)};
            m["TextBox"]["ClearTextOnFocus"] = {ValueType::Bool, Variant::make_bool(true)};
            m["TextBox"]["TextEditable"] = {ValueType::Bool, Variant::make_bool(true)};
            m["TextBox"]["MultiLine"] = {ValueType::Bool, Variant::make_bool(false)};
            auto image = gui_object_props();
            image["Image"] = {ValueType::ContentId, content("")};
            image["ImageColor3"] = {ValueType::Color3, color(1, 1, 1)};
            image["ImageTransparency"] = {ValueType::Float32, f32(0)};
            image["ScaleType"] = {ValueType::Enum, Variant::make_enum(0)};
            image["SliceScale"] = {ValueType::Float32, f32(1)};
            m["ImageLabel"] = image;
            m["ImageButton"] = image;
            m["ImageButton"]["AutoButtonColor"] = {ValueType::Bool, Variant::make_bool(true)};
            auto scroll = gui_object_props();
            scroll["CanvasPosition"] = {ValueType::Vector2, vec2(0, 0)};
            scroll["ScrollingEnabled"] = {ValueType::Bool, Variant::make_bool(true)};
            scroll["ScrollBarThickness"] = {ValueType::Int32, i32(12)};
            m["ScrollingFrame"] = scroll;
            m["ScreenGui"] = {
                {"DisplayOrder", {ValueType::Int32, i32(0)}},
                {"Enabled", {ValueType::Bool, Variant::make_bool(true)}},
                {"IgnoreGuiInset", {ValueType::Bool, Variant::make_bool(false)}},
                {"ResetOnSpawn", {ValueType::Bool, Variant::make_bool(true)}},
                {"ZIndexBehavior", {ValueType::Enum, Variant::make_enum(0)}},
            };
            m["SurfaceGui"] = {
                {"Adornee", {ValueType::Ref, [] { Variant v; v.type = ValueType::Ref; return v; }()}},
                {"AlwaysOnTop", {ValueType::Bool, Variant::make_bool(false)}},
                {"Enabled", {ValueType::Bool, Variant::make_bool(true)}},
                {"Face", {ValueType::Enum, Variant::make_enum(5)}},
                {"LightInfluence", {ValueType::Float32, f32(1)}},
                {"ZIndexBehavior", {ValueType::Enum, Variant::make_enum(0)}},
            };
            m["BillboardGui"] = m["SurfaceGui"];
            m["UIStroke"] = {
                {"Color", {ValueType::Color3, color(0, 0, 0)}},
                {"Thickness", {ValueType::Float32, f32(1)}},
                {"Transparency", {ValueType::Float32, f32(0)}},
                {"Enabled", {ValueType::Bool, Variant::make_bool(true)}},
                {"ApplyStrokeMode", {ValueType::Enum, Variant::make_enum(0)}},
                {"LineJoinMode", {ValueType::Enum, Variant::make_enum(0)}},
            };
            m["UICorner"] = {{"CornerRadius", {ValueType::UDim, [] { Variant v; v.type = ValueType::UDim; v.complex_value = Json::array(); v.complex_value.push(Json(0.0)); v.complex_value.push(Json(8.0)); return v; }()}}};
            m["UIPadding"] = {
                {"PaddingLeft", {ValueType::UDim, m["UICorner"]["CornerRadius"].default_value}},
                {"PaddingRight", {ValueType::UDim, m["UICorner"]["CornerRadius"].default_value}},
                {"PaddingTop", {ValueType::UDim, m["UICorner"]["CornerRadius"].default_value}},
                {"PaddingBottom", {ValueType::UDim, m["UICorner"]["CornerRadius"].default_value}},
            };
            // Normalise padding defaults to zero offset (UDim 0,0).
            for (auto& [k, v] : m["UIPadding"]) { (void)k; v.default_value.complex_value = Json::array(); v.default_value.complex_value.push(Json(0.0)); v.default_value.complex_value.push(Json(0.0)); }
            m["UIScale"] = {{"Scale", {ValueType::Float64, Variant::make_float64(1)}}};
            m["UIAspectRatioConstraint"] = {
                {"AspectRatio", {ValueType::Float64, Variant::make_float64(1)}},
                {"AspectType", {ValueType::Enum, Variant::make_enum(0)}},
                {"DominantAxis", {ValueType::Enum, Variant::make_enum(0)}},
            };
            m["UISizeConstraint"] = {
                {"MaxSize", {ValueType::Vector2, vec2(1e9, 1e9)}},
                {"MinSize", {ValueType::Vector2, vec2(0, 0)}},
            };
            m["UITextSizeConstraint"] = {
                {"MaxTextSize", {ValueType::Int32, i32(100)}},
                {"MinTextSize", {ValueType::Int32, i32(1)}},
            };
            m["UIListLayout"] = {
                {"FillDirection", {ValueType::Enum, Variant::make_enum(0)}},
                {"HorizontalAlignment", {ValueType::Enum, Variant::make_enum(0)}},
                {"VerticalAlignment", {ValueType::Enum, Variant::make_enum(0)}},
                {"SortOrder", {ValueType::Enum, Variant::make_enum(1)}},
                {"Wraps", {ValueType::Bool, Variant::make_bool(false)}},
            };
            m["UIGridLayout"] = m["UIListLayout"];
            m["UIPageLayout"] = m["UIListLayout"];
            m["UIGradient"] = {
                {"Color", {ValueType::ColorSequence, default_color_sequence()}},
                {"Offset", {ValueType::Vector2, vec2(0, 0)}},
                {"Rotation", {ValueType::Float32, f32(0)}},
                {"Transparency", {ValueType::NumberSequence, default_number_sequence(0)}},
                {"Enabled", {ValueType::Bool, Variant::make_bool(true)}},
            };
            return m;
        }();
        auto git = gui.find(class_name);
        if (git != gui.end()) return &git->second;
    }
    return part_like(class_name);
}

bool is_known_class(const std::string& class_name) { return known_classes().count(class_name) > 0; }


} // namespace blox
