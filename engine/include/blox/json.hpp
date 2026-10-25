#pragma once
#include <cstdint>
#include <map>
#include <memory>
#include <string>
#include <variant>
#include <vector>

namespace blox {

// Minimal JSON value used for the engine IPC protocol. Objects preserve
// insertion order so snapshots are deterministic.
class Json {
public:
    using Array = std::vector<Json>;
    using Object = std::vector<std::pair<std::string, Json>>;
    using Storage = std::variant<std::nullptr_t, bool, int64_t, double, std::string,
                                 std::shared_ptr<Array>, std::shared_ptr<Object>>;

    Json() : value_(nullptr) {}
    Json(std::nullptr_t) : value_(nullptr) {}
    Json(bool v) : value_(v) {}
    Json(int v) : value_(static_cast<int64_t>(v)) {}
    Json(int64_t v) : value_(v) {}
    Json(double v) : value_(v) {}
    Json(const char* v) : value_(std::string(v)) {}
    Json(std::string v) : value_(std::move(v)) {}
    Json(Array v) : value_(std::make_shared<Array>(std::move(v))) {}
    Json(Object v) : value_(std::make_shared<Object>(std::move(v))) {}

    static Json array() { return Json(Array{}); }
    static Json object() { return Json(Object{}); }

    bool is_null() const { return std::holds_alternative<std::nullptr_t>(value_); }
    bool is_bool() const { return std::holds_alternative<bool>(value_); }
    bool is_number() const {
        return std::holds_alternative<int64_t>(value_) || std::holds_alternative<double>(value_);
    }
    bool is_int() const { return std::holds_alternative<int64_t>(value_); }
    bool is_string() const { return std::holds_alternative<std::string>(value_); }
    bool is_array() const { return std::holds_alternative<std::shared_ptr<Array>>(value_); }
    bool is_object() const { return std::holds_alternative<std::shared_ptr<Object>>(value_); }

    bool as_bool(bool fallback = false) const {
        if (auto* p = std::get_if<bool>(&value_)) return *p;
        return fallback;
    }
    int64_t as_int(int64_t fallback = 0) const {
        if (auto* p = std::get_if<int64_t>(&value_)) return *p;
        if (auto* p = std::get_if<double>(&value_)) return static_cast<int64_t>(*p);
        return fallback;
    }
    double as_double(double fallback = 0.0) const {
        if (auto* p = std::get_if<double>(&value_)) return *p;
        if (auto* p = std::get_if<int64_t>(&value_)) return static_cast<double>(*p);
        return fallback;
    }
    const std::string& as_string() const {
        static const std::string empty;
        if (auto* p = std::get_if<std::string>(&value_)) return *p;
        return empty;
    }
    const Array& as_array() const {
        static const Array empty;
        if (auto* p = std::get_if<std::shared_ptr<Array>>(&value_)) return **p;
        return empty;
    }
    const Object& as_object() const {
        static const Object empty;
        if (auto* p = std::get_if<std::shared_ptr<Object>>(&value_)) return **p;
        return empty;
    }

    const Json* find(const std::string& key) const {
        if (!is_object()) return nullptr;
        for (const auto& [k, v] : as_object())
            if (k == key) return &v;
        return nullptr;
    }
    bool contains(const std::string& key) const { return find(key) != nullptr; }
    void set(const std::string& key, Json v) {
        if (!is_object()) value_ = std::make_shared<Object>();
        auto& obj = *std::get<std::shared_ptr<Object>>(value_);
        for (auto& [k, existing] : obj)
            if (k == key) { existing = std::move(v); return; }
        obj.emplace_back(key, std::move(v));
    }
    void push(Json v) {
        if (!is_array()) value_ = std::make_shared<Array>();
        std::get<std::shared_ptr<Array>>(value_)->push_back(std::move(v));
    }

    std::string dump(int indent = -1) const;

    // Parses JSON text. On failure returns false and sets `error`.
    static bool parse(const std::string& text, Json& out, std::string& error);

private:
    Storage value_;
};

} // namespace blox
