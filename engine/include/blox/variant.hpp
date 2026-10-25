#pragma once
#include <array>
#include <cstdint>
#include <string>
#include <vector>

#include "blox/json.hpp"

namespace blox {

// Typed property value. The JSON form is the IPC/file-conversion contract
// shared with the Rust CLI:
//   {"type":"Bool","value":true}
//   {"type":"Int32"|"Int64","value":123}
//   {"type":"Float32"|"Float64","value":1.5}
//   {"type":"String"|"ContentId","value":"..."}
//   {"type":"BinaryString","value":"<base64>"}
//   {"type":"Vector3","value":[x,y,z]}
//   {"type":"Color3","value":[r,g,b]}
//   {"type":"CFrame","value":{"position":[x,y,z],"rotation":[r00,r01,r02,r10,...]}}
//   {"type":"Enum"|"BrickColor","value":123}
enum class ValueType {
    Bool, Int32, Int64, Float32, Float64, String, BinaryString,
    Vector3, Color3, CFrame, Enum, BrickColor, ContentId, Tags, Attributes,
    // Phase 2 extensions. JSON shapes (arrays/objects carried in complex_value):
    //   Vector2 [x,y]            Vector2int16 [x,y]        Vector3int16 [x,y,z]
    //   UDim [scale,offset]      UDim2 [xs,xo,ys,yo]       NumberRange [min,max]
    //   Rect [x0,y0,x1,y1]       Ray {"origin":[3],"direction":[3]}
    //   Faces/Axes: integer bitmask (int_value)
    //   PhysicalProperties: "Default" or {"density":f,"friction":f,"elasticity":f,
    //     "frictionWeight":f,"elasticityWeight":f,"acousticAbsorption":f}
    //   ColorSequence [{"time":f,"color":[r,g,b]}]
    //   NumberSequence [{"time":f,"value":f,"envelope":f}]
    //   EnumItem {"enum":"Material","value":512}
    Vector2, Vector2int16, Vector3int16, UDim, UDim2, NumberRange, Rect, Ray,
    Faces, Axes, PhysicalProperties, ColorSequence, NumberSequence, EnumItem,
    Font, // value: {"family":str,"weight":int,"style":str}
    Ref // value: child-index path of the target instance ("0/2/1"), "" = none
};

inline const char* value_type_name(ValueType t) {
    switch (t) {
        case ValueType::Bool: return "Bool";
        case ValueType::Int32: return "Int32";
        case ValueType::Int64: return "Int64";
        case ValueType::Float32: return "Float32";
        case ValueType::Float64: return "Float64";
        case ValueType::String: return "String";
        case ValueType::BinaryString: return "BinaryString";
        case ValueType::Vector3: return "Vector3";
        case ValueType::Color3: return "Color3";
        case ValueType::CFrame: return "CFrame";
        case ValueType::Enum: return "Enum";
        case ValueType::BrickColor: return "BrickColor";
        case ValueType::ContentId: return "ContentId";
        case ValueType::Tags: return "Tags";
        case ValueType::Attributes: return "Attributes";
        case ValueType::Vector2: return "Vector2";
        case ValueType::Vector2int16: return "Vector2int16";
        case ValueType::Vector3int16: return "Vector3int16";
        case ValueType::UDim: return "UDim";
        case ValueType::UDim2: return "UDim2";
        case ValueType::NumberRange: return "NumberRange";
        case ValueType::Rect: return "Rect";
        case ValueType::Ray: return "Ray";
        case ValueType::Faces: return "Faces";
        case ValueType::Axes: return "Axes";
        case ValueType::PhysicalProperties: return "PhysicalProperties";
        case ValueType::ColorSequence: return "ColorSequence";
        case ValueType::NumberSequence: return "NumberSequence";
        case ValueType::EnumItem: return "EnumItem";
        case ValueType::Ref: return "Ref";
        case ValueType::Font: return "Font";
    }
    return "Unknown";
}

inline bool value_type_from_name(const std::string& name, ValueType& out) {
    using V = ValueType;
    if (name == "Bool") out = V::Bool;
    else if (name == "Int32") out = V::Int32;
    else if (name == "Int64") out = V::Int64;
    else if (name == "Float32") out = V::Float32;
    else if (name == "Float64") out = V::Float64;
    else if (name == "String") out = V::String;
    else if (name == "BinaryString") out = V::BinaryString;
    else if (name == "Vector3") out = V::Vector3;
    else if (name == "Color3") out = V::Color3;
    else if (name == "CFrame") out = V::CFrame;
    else if (name == "Enum") out = V::Enum;
    else if (name == "BrickColor") out = V::BrickColor;
    else if (name == "ContentId") out = V::ContentId;
    else if (name == "Tags") out = V::Tags;
    else if (name == "Attributes") out = V::Attributes;
    else if (name == "Vector2") out = V::Vector2;
    else if (name == "Vector2int16") out = V::Vector2int16;
    else if (name == "Vector3int16") out = V::Vector3int16;
    else if (name == "UDim") out = V::UDim;
    else if (name == "UDim2") out = V::UDim2;
    else if (name == "NumberRange") out = V::NumberRange;
    else if (name == "Rect") out = V::Rect;
    else if (name == "Ray") out = V::Ray;
    else if (name == "Faces") out = V::Faces;
    else if (name == "Axes") out = V::Axes;
    else if (name == "PhysicalProperties") out = V::PhysicalProperties;
    else if (name == "ColorSequence") out = V::ColorSequence;
    else if (name == "NumberSequence") out = V::NumberSequence;
    else if (name == "EnumItem") out = V::EnumItem;
    else if (name == "Ref") out = V::Ref;
    else if (name == "Font") out = V::Font;
    else return false;
    return true;
}

struct Variant {
    ValueType type = ValueType::String;
    bool bool_value = false;
    int64_t int_value = 0;
    double float_value = 0.0;
    std::string string_value;              // String, ContentId, or base64 for BinaryString
    std::array<double, 3> vec3{0, 0, 0};   // Vector3 / Color3
    std::array<double, 3> position{0, 0, 0};
    std::array<double, 9> rotation{1, 0, 0, 0, 1, 0, 0, 0, 1}; // row-major
    Json complex_value; // Tags: JSON array of strings. Attributes: JSON object of nested variants.

    static Variant make_bool(bool v) { Variant r; r.type = ValueType::Bool; r.bool_value = v; return r; }
    static Variant make_int64(int64_t v) { Variant r; r.type = ValueType::Int64; r.int_value = v; return r; }
    static Variant make_float64(double v) { Variant r; r.type = ValueType::Float64; r.float_value = v; return r; }
    static Variant make_string(std::string v) { Variant r; r.type = ValueType::String; r.string_value = std::move(v); return r; }
    static Variant make_enum(int64_t v) { Variant r; r.type = ValueType::Enum; r.int_value = v; return r; }
    static Variant make_vec3(double x, double y, double z) { Variant r; r.type = ValueType::Vector3; r.vec3 = {x, y, z}; return r; }
    static Variant make_color3(double r_, double g, double b) { Variant r; r.type = ValueType::Color3; r.vec3 = {r_, g, b}; return r; }
    static Variant make_cframe(double x, double y, double z) { Variant r; r.type = ValueType::CFrame; r.position = {x, y, z}; return r; }

    Json to_json() const {
        Json obj = Json::object();
        obj.set("type", Json(value_type_name(type)));
        switch (type) {
            case ValueType::Bool: obj.set("value", Json(bool_value)); break;
            case ValueType::Int32: case ValueType::Int64:
            case ValueType::Enum: case ValueType::BrickColor:
                obj.set("value", Json(int_value)); break;
            case ValueType::Float32: case ValueType::Float64:
                obj.set("value", Json(float_value)); break;
            case ValueType::String: case ValueType::BinaryString: case ValueType::ContentId:
            case ValueType::Ref:
                obj.set("value", Json(string_value)); break;
            case ValueType::Vector3: case ValueType::Color3: {
                Json a = Json::array();
                for (double d : vec3) a.push(Json(d));
                obj.set("value", std::move(a));
                break;
            }
            case ValueType::Tags: case ValueType::Attributes:
            case ValueType::Vector2: case ValueType::Vector2int16: case ValueType::Vector3int16:
            case ValueType::UDim: case ValueType::UDim2: case ValueType::NumberRange:
            case ValueType::Rect: case ValueType::Ray: case ValueType::PhysicalProperties:
            case ValueType::ColorSequence: case ValueType::NumberSequence: case ValueType::EnumItem:
            case ValueType::Font:
                obj.set("value", complex_value); break;
            case ValueType::Faces: case ValueType::Axes:
                obj.set("value", Json(int_value)); break;
            case ValueType::CFrame: {
                Json v = Json::object();
                Json p = Json::array();
                for (double d : position) p.push(Json(d));
                Json r = Json::array();
                for (double d : rotation) r.push(Json(d));
                v.set("position", std::move(p));
                v.set("rotation", std::move(r));
                obj.set("value", std::move(v));
                break;
            }
        }
        return obj;
    }

    static bool from_json(const Json& j, Variant& out, std::string& error) {
        const Json* type_j = j.find("type");
        const Json* val_j = j.find("value");
        if (!type_j || !type_j->is_string() || !val_j) { error = "variant must have string 'type' and 'value'"; return false; }
        ValueType t;
        if (!value_type_from_name(type_j->as_string(), t)) { error = "unsupported value type '" + type_j->as_string() + "'"; return false; }
        out = Variant{};
        out.type = t;
        switch (t) {
            case ValueType::Bool:
                if (!val_j->is_bool()) { error = "Bool value must be a boolean"; return false; }
                out.bool_value = val_j->as_bool(); break;
            case ValueType::Int32: case ValueType::Int64:
            case ValueType::Enum: case ValueType::BrickColor:
                if (!val_j->is_number()) { error = "integer value must be a number"; return false; }
                out.int_value = val_j->as_int(); break;
            case ValueType::Float32: case ValueType::Float64:
                if (!val_j->is_number()) { error = "float value must be a number"; return false; }
                out.float_value = val_j->as_double(); break;
            case ValueType::String: case ValueType::BinaryString: case ValueType::ContentId:
            case ValueType::Ref:
                if (!val_j->is_string()) { error = "string value must be a string"; return false; }
                out.string_value = val_j->as_string(); break;
            case ValueType::Tags:
                if (!val_j->is_array()) { error = "Tags value must be an array of strings"; return false; }
                out.complex_value = *val_j; break;
            case ValueType::Attributes:
                if (!val_j->is_object()) { error = "Attributes value must be an object"; return false; }
                out.complex_value = *val_j; break;
            case ValueType::Faces: case ValueType::Axes:
                if (!val_j->is_number()) { error = "Faces/Axes value must be an integer bitmask"; return false; }
                out.int_value = val_j->as_int(); break;
            case ValueType::Vector2: case ValueType::Vector2int16: case ValueType::Vector3int16:
            case ValueType::UDim: case ValueType::UDim2: case ValueType::NumberRange:
            case ValueType::Rect: case ValueType::ColorSequence: case ValueType::NumberSequence:
                if (!val_j->is_array()) { error = std::string(value_type_name(t)) + " value must be an array"; return false; }
                out.complex_value = *val_j; break;
            case ValueType::Ray: case ValueType::EnumItem: case ValueType::Font:
                if (!val_j->is_object()) { error = std::string(value_type_name(t)) + " value must be an object"; return false; }
                out.complex_value = *val_j; break;
            case ValueType::PhysicalProperties:
                if (!val_j->is_object() && !val_j->is_string() && !val_j->is_array()) { error = "PhysicalProperties value must be \"Default\" or a 6-number array"; return false; }
                out.complex_value = *val_j; break;
            case ValueType::Vector3: case ValueType::Color3: {
                if (!val_j->is_array() || val_j->as_array().size() != 3) { error = "Vector3/Color3 value must be [x, y, z]"; return false; }
                for (size_t i = 0; i < 3; ++i) out.vec3[i] = val_j->as_array()[i].as_double();
                break;
            }
            case ValueType::CFrame: {
                const Json* p = val_j->find("position");
                const Json* r = val_j->find("rotation");
                if (!p || !p->is_array() || p->as_array().size() != 3) { error = "CFrame.position must be [x, y, z]"; return false; }
                for (size_t i = 0; i < 3; ++i) out.position[i] = p->as_array()[i].as_double();
                if (r) {
                    if (!r->is_array() || r->as_array().size() != 9) { error = "CFrame.rotation must be 9 numbers"; return false; }
                    for (size_t i = 0; i < 9; ++i) out.rotation[i] = r->as_array()[i].as_double();
                }
                break;
            }
        }
        return true;
    }

    bool operator==(const Variant& o) const {
        return type == o.type && bool_value == o.bool_value && int_value == o.int_value &&
               float_value == o.float_value && string_value == o.string_value &&
               vec3 == o.vec3 && position == o.position && rotation == o.rotation &&
               complex_value.dump() == o.complex_value.dump();
    }
};

} // namespace blox
