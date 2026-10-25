#include "blox/core/json.hpp"

#include <charconv>
#include <cmath>
#include <cstdio>

namespace blox {
namespace {

void dump_into(const Json& j, std::string& out, int indent, int depth) {
    auto newline_indent = [&](int d) {
        if (indent < 0) return;
        out.push_back('\n');
        out.append(static_cast<size_t>(indent * d), ' ');
    };
    if (j.is_null()) { out += "null"; return; }
    if (j.is_bool()) { out += j.as_bool() ? "true" : "false"; return; }
    if (j.is_int()) { out += std::to_string(j.as_int()); return; }
    if (j.is_number()) {
        double d = j.as_double();
        if (std::isfinite(d) && d == std::floor(d) && std::abs(d) < 1e15) {
            char buf[32];
            std::snprintf(buf, sizeof(buf), "%.1f", d);
            out += buf;
        } else {
            char buf[40];
            std::snprintf(buf, sizeof(buf), "%.17g", d);
            out += buf;
        }
        return;
    }
    if (j.is_string()) {
        out.push_back('"');
        for (char c : j.as_string()) {
            switch (c) {
                case '"': out += "\\\""; break;
                case '\\': out += "\\\\"; break;
                case '\n': out += "\\n"; break;
                case '\r': out += "\\r"; break;
                case '\t': out += "\\t"; break;
                case '\b': out += "\\b"; break;
                case '\f': out += "\\f"; break;
                default:
                    if (static_cast<unsigned char>(c) < 0x20) {
                        char buf[8];
                        std::snprintf(buf, sizeof(buf), "\\u%04x", c);
                        out += buf;
                    } else {
                        out.push_back(c);
                    }
            }
        }
        out.push_back('"');
        return;
    }
    if (j.is_array()) {
        const auto& a = j.as_array();
        if (a.empty()) { out += "[]"; return; }
        out.push_back('[');
        bool first = true;
        for (const auto& el : a) {
            if (!first) out.push_back(',');
            first = false;
            newline_indent(depth + 1);
            dump_into(el, out, indent, depth + 1);
        }
        newline_indent(depth);
        out.push_back(']');
        return;
    }
    const auto& o = j.as_object();
    if (o.empty()) { out += "{}"; return; }
    out.push_back('{');
    bool first = true;
    for (const auto& [k, v] : o) {
        if (!first) out.push_back(',');
        first = false;
        newline_indent(depth + 1);
        Json key(k);
        dump_into(key, out, indent, depth + 1);
        out += (indent < 0 ? ":" : ": ");
        dump_into(v, out, indent, depth + 1);
    }
    newline_indent(depth);
    out.push_back('}');
}

struct Parser {
    const std::string& s;
    size_t pos = 0;
    std::string& error;

    void skip_ws() {
        while (pos < s.size() && (s[pos] == ' ' || s[pos] == '\t' || s[pos] == '\n' || s[pos] == '\r')) ++pos;
    }
    bool fail(const std::string& msg) { error = msg + " at offset " + std::to_string(pos); return false; }
    bool consume(char c) {
        skip_ws();
        if (pos < s.size() && s[pos] == c) { ++pos; return true; }
        return false;
    }
    bool parse_value(Json& out) {
        skip_ws();
        if (pos >= s.size()) return fail("unexpected end of input");
        char c = s[pos];
        if (c == '{') return parse_object(out);
        if (c == '[') return parse_array(out);
        if (c == '"') { std::string v; if (!parse_string(v)) return false; out = Json(std::move(v)); return true; }
        if (c == 't') { if (s.compare(pos, 4, "true") == 0) { pos += 4; out = Json(true); return true; } return fail("invalid literal"); }
        if (c == 'f') { if (s.compare(pos, 5, "false") == 0) { pos += 5; out = Json(false); return true; } return fail("invalid literal"); }
        if (c == 'n') { if (s.compare(pos, 4, "null") == 0) { pos += 4; out = Json(); return true; } return fail("invalid literal"); }
        return parse_number(out);
    }
    bool parse_object(Json& out) {
        ++pos; // {
        Json obj = Json::object();
        skip_ws();
        if (pos < s.size() && s[pos] == '}') { ++pos; out = std::move(obj); return true; }
        while (true) {
            skip_ws();
            std::string key;
            if (pos >= s.size() || s[pos] != '"' || !parse_string(key)) return fail("expected object key");
            if (!consume(':')) return fail("expected ':'");
            Json val;
            if (!parse_value(val)) return false;
            obj.set(key, std::move(val));
            skip_ws();
            if (pos < s.size() && s[pos] == ',') { ++pos; continue; }
            if (pos < s.size() && s[pos] == '}') { ++pos; out = std::move(obj); return true; }
            return fail("expected ',' or '}'");
        }
    }
    bool parse_array(Json& out) {
        ++pos; // [
        Json arr = Json::array();
        skip_ws();
        if (pos < s.size() && s[pos] == ']') { ++pos; out = std::move(arr); return true; }
        while (true) {
            Json val;
            if (!parse_value(val)) return false;
            arr.push(std::move(val));
            skip_ws();
            if (pos < s.size() && s[pos] == ',') { ++pos; continue; }
            if (pos < s.size() && s[pos] == ']') { ++pos; out = std::move(arr); return true; }
            return fail("expected ',' or ']'");
        }
    }
    bool parse_string(std::string& out) {
        if (s[pos] != '"') return fail("expected string");
        ++pos;
        while (pos < s.size()) {
            char c = s[pos++];
            if (c == '"') return true;
            if (c == '\\') {
                if (pos >= s.size()) break;
                char e = s[pos++];
                switch (e) {
                    case '"': out.push_back('"'); break;
                    case '\\': out.push_back('\\'); break;
                    case '/': out.push_back('/'); break;
                    case 'b': out.push_back('\b'); break;
                    case 'f': out.push_back('\f'); break;
                    case 'n': out.push_back('\n'); break;
                    case 'r': out.push_back('\r'); break;
                    case 't': out.push_back('\t'); break;
                    case 'u': {
                        if (pos + 4 > s.size()) return fail("bad \\u escape");
                        auto hex = [&](size_t at) -> int {
                            int v = 0;
                            for (int i = 0; i < 4; ++i) {
                                char h = s[at + i];
                                v <<= 4;
                                if (h >= '0' && h <= '9') v += h - '0';
                                else if (h >= 'a' && h <= 'f') v += h - 'a' + 10;
                                else if (h >= 'A' && h <= 'F') v += h - 'A' + 10;
                                else return -1;
                            }
                            return v;
                        };
                        int cp = hex(pos);
                        if (cp < 0) return fail("bad \\u escape");
                        pos += 4;
                        if (cp >= 0xD800 && cp <= 0xDBFF && pos + 6 <= s.size() && s[pos] == '\\' && s[pos + 1] == 'u') {
                            int lo = hex(pos + 2);
                            if (lo >= 0xDC00 && lo <= 0xDFFF) {
                                cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                pos += 6;
                            }
                        }
                        if (cp < 0x80) out.push_back(static_cast<char>(cp));
                        else if (cp < 0x800) {
                            out.push_back(static_cast<char>(0xC0 | (cp >> 6)));
                            out.push_back(static_cast<char>(0x80 | (cp & 0x3F)));
                        } else if (cp < 0x10000) {
                            out.push_back(static_cast<char>(0xE0 | (cp >> 12)));
                            out.push_back(static_cast<char>(0x80 | ((cp >> 6) & 0x3F)));
                            out.push_back(static_cast<char>(0x80 | (cp & 0x3F)));
                        } else {
                            out.push_back(static_cast<char>(0xF0 | (cp >> 18)));
                            out.push_back(static_cast<char>(0x80 | ((cp >> 12) & 0x3F)));
                            out.push_back(static_cast<char>(0x80 | ((cp >> 6) & 0x3F)));
                            out.push_back(static_cast<char>(0x80 | (cp & 0x3F)));
                        }
                        break;
                    }
                    default: return fail("bad escape");
                }
            } else {
                out.push_back(c);
            }
        }
        return fail("unterminated string");
    }
    bool parse_number(Json& out) {
        size_t start = pos;
        if (pos < s.size() && s[pos] == '-') ++pos;
        bool is_float = false;
        while (pos < s.size() && (isdigit(static_cast<unsigned char>(s[pos])) || s[pos] == '.' || s[pos] == 'e' || s[pos] == 'E' || s[pos] == '+' || s[pos] == '-')) {
            if (s[pos] == '.' || s[pos] == 'e' || s[pos] == 'E') is_float = true;
            ++pos;
        }
        if (pos == start) return fail("invalid number");
        std::string text = s.substr(start, pos - start);
        if (!is_float) {
            int64_t v = 0;
            auto [ptr, ec] = std::from_chars(text.data(), text.data() + text.size(), v);
            if (ec == std::errc() && ptr == text.data() + text.size()) { out = Json(v); return true; }
        }
        try {
            out = Json(std::stod(text));
        } catch (...) { return fail("invalid number"); }
        return true;
    }
};

} // namespace

std::string Json::dump(int indent) const {
    std::string out;
    dump_into(*this, out, indent, 0);
    return out;
}

bool Json::parse(const std::string& text, Json& out, std::string& error) {
    Parser p{text, 0, error};
    if (!p.parse_value(out)) return false;
    p.skip_ws();
    if (p.pos != text.size()) { error = "trailing data at offset " + std::to_string(p.pos); return false; }
    return true;
}

} // namespace blox
