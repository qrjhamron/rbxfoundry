#include "blox/ui.hpp"

#include <algorithm>
#include <cmath>
#include <functional>
#include <png.h>
#include <map>
#include <set>
#include <cstdlib>
#include <unistd.h>

#ifdef BLOX_HAVE_FREETYPE
#include <ft2build.h>
#include FT_FREETYPE_H
#endif

namespace blox {
namespace {

bool file_exists(const std::string& path) {
    FILE* f = fopen(path.c_str(), "rb");
    if (f) { fclose(f); return true; }
    return false;
}

std::string find_default_font() {
    // 1. Explicit override (also used by release tarballs).
    if (const char* env = std::getenv("BLOX_FONT")) {
        if (env[0] && file_exists(env)) return env;
    }
    // 2. Fonts bundled next to the engine binary (release layout:
    //    <bin>/fonts/DejaVuSans.ttf or <bin>/../share/blox/fonts/).
    {
        char exe[4096];
        ssize_t n = readlink("/proc/self/exe", exe, sizeof(exe) - 1);
        if (n > 0) {
            exe[n] = '\0';
            std::string dir(exe);
            size_t slash = dir.find_last_of('/');
            if (slash != std::string::npos) {
                dir = dir.substr(0, slash);
                for (const std::string& rel : {"/fonts/DejaVuSans.ttf", "/../share/blox/fonts/DejaVuSans.ttf"}) {
                    if (file_exists(dir + rel)) return dir + rel;
                }
            }
        }
    }
    // 3. System fonts.
    const char* candidates[] = {
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    };
    for (const char* path : candidates) {
        if (file_exists(path)) return path;
    }
    return "";
}

} // namespace

#ifdef BLOX_HAVE_FREETYPE
namespace {

struct FontFace {
    FT_Face face = nullptr;
};

FT_Library& ft_library() {
    static FT_Library lib = nullptr;
    if (!lib) FT_Init_FreeType(&lib);
    return lib;
}

FT_Face load_face(const std::string& font_path, std::string& error) {
    std::string path = font_path.empty() ? find_default_font() : font_path;
    if (path.empty()) {
        error = "no usable TTF font found (install fonts-dejavu-core or pass a font path)";
        return nullptr;
    }
    FT_Face face = nullptr;
    if (FT_New_Face(ft_library(), path.c_str(), 0, &face) != 0) {
        error = "failed to load font '" + path + "'";
        return nullptr;
    }
    return face;
}

// Decodes UTF-8 into codepoints (Phase 19). Invalid bytes decode to
// U+FFFD and consume one byte, so malformed text degrades visibly
// instead of wedging the layout engine. Text storage stays UTF-8
// everywhere; only interpretation changed.
std::vector<char32_t> decode_utf8(const std::string& text) {
    std::vector<char32_t> out;
    size_t i = 0;
    while (i < text.size()) {
        unsigned char lead = static_cast<unsigned char>(text[i]);
        if (lead < 0x80) { out.push_back(lead); ++i; continue; }
        int len = (lead >= 0xF0) ? 4 : (lead >= 0xE0) ? 3 : (lead >= 0xC0) ? 2 : 0;
        if (len == 0 || i + static_cast<size_t>(len) > text.size()) {
            out.push_back(0xFFFD);
            ++i;
            continue;
        }
        char32_t cp = lead & (len == 2 ? 0x1F : len == 3 ? 0x0F : 0x07);
        bool valid = true;
        for (int k = 1; k < len; ++k) {
            unsigned char cont = static_cast<unsigned char>(text[i + k]);
            if ((cont & 0xC0) != 0x80) { valid = false; break; }
            cp = (cp << 6) | (cont & 0x3F);
        }
        // Reject overlong encodings and surrogates.
        if (valid && ((len == 2 && cp < 0x80) || (len == 3 && cp < 0x800) ||
                      (len == 4 && (cp < 0x10000 || cp > 0x10FFFF)) ||
                      (cp >= 0xD800 && cp <= 0xDFFF))) {
            valid = false;
        }
        out.push_back(valid ? cp : 0xFFFD);
        i += valid ? static_cast<size_t>(len) : 1;
    }
    return out;
}

bool is_cjk_codepoint(char32_t cp) {
    return (cp >= 0x2E80 && cp <= 0x9FFF) || (cp >= 0xF900 && cp <= 0xFAFF) ||
           (cp >= 0x20000 && cp <= 0x2FA1F);
}

// Ordered fallback chain: the requested font first, then the locally
// installed DejaVu family. A codepoint is rendered by the first face
// that has a glyph for it; codepoints no face covers are reported by
// callers and drawn as .notdef boxes — never silently dropped.
struct FaceChain {
    std::vector<FT_Face> faces;
    FT_Face pick(char32_t cp) const {
        for (FT_Face face : faces) {
            if (FT_Get_Char_Index(face, cp) != 0) return face;
        }
        return nullptr;
    }
    void set_size(float size_px) {
        for (FT_Face face : faces) {
            FT_Set_Pixel_Sizes(face, 0, static_cast<FT_UInt>(std::max(size_px, 1.0f)));
        }
    }
    void dispose() {
        for (FT_Face face : faces) FT_Done_Face(face);
        faces.clear();
    }
};

FaceChain load_chain(const std::string& primary_path) {
    FaceChain chain;
    std::vector<std::string> candidates;
    candidates.push_back(primary_path.empty() ? find_default_font() : primary_path);
    candidates.push_back("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf");
    candidates.push_back("/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf");
    candidates.push_back("/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf");
    candidates.push_back("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf");
    std::string error;
    std::vector<std::string> loaded_paths;
    for (const std::string& path : candidates) {
        if (path.empty()) continue;
        if (std::find(loaded_paths.begin(), loaded_paths.end(), path) != loaded_paths.end()) continue;
        FT_Face face = load_face(path, error);
        if (face) {
            chain.faces.push_back(face);
            loaded_paths.push_back(path);
        }
    }
    return chain;
}

// Packs glyphs into a simple shelf atlas, appending quads for each glyph.
struct AtlasPacker {
    std::vector<uint8_t>& rgba;
    uint32_t width;
    uint32_t x = 1, y = 1, row_h = 0;
    AtlasPacker(std::vector<uint8_t>& r, uint32_t w) : rgba(r), width(w) {}
    // Returns UV rect for a glyph bitmap; `bmp` is 8-bit coverage.
    std::array<float, 4> add(const unsigned char* bmp, uint32_t w, uint32_t h, uint32_t pitch) {
        if (x + w + 2 > width) { x = 1; y += row_h + 2; row_h = 0; }
        uint32_t needed_h = y + h + 2;
        if (rgba.size() < static_cast<size_t>(width) * needed_h * 4) {
            rgba.resize(static_cast<size_t>(width) * needed_h * 4, 0);
        }
        for (uint32_t row = 0; row < h; ++row) {
            for (uint32_t col = 0; col < w; ++col) {
                size_t at = (static_cast<size_t>(y + row) * width + (x + col)) * 4;
                rgba[at + 0] = 255;
                rgba[at + 1] = 255;
                rgba[at + 2] = 255;
                rgba[at + 3] = bmp[static_cast<size_t>(row) * pitch + col];
            }
        }
        // Pixel-space rect; callers normalise (v by the final atlas
        // height, which can grow beyond the initial allocation).
        std::array<float, 4> uv{static_cast<float>(x), static_cast<float>(y),
                                static_cast<float>(x + w), static_cast<float>(y + h)};
        x += w + 2;
        row_h = std::max(row_h, h);
        return uv;
    }
    uint32_t height() const { return y + row_h + 2; }
};

} // namespace

float measure_text(const std::string& text, float size_px, const std::string& font_path) {
    FaceChain chain = load_chain(font_path);
    if (chain.faces.empty()) return 0.0f;
    chain.set_size(size_px);
    FT_Face primary = chain.faces.front();
    float width = 0;
    for (char32_t cp : decode_utf8(text)) {
        FT_Face face = chain.pick(cp);
        if (!face) {
            // No font covers this codepoint: reserve .notdef width.
            if (FT_Load_Char(primary, 0, FT_LOAD_DEFAULT) == 0)
                width += static_cast<float>(primary->glyph->advance.x >> 6);
            continue;
        }
        FT_UInt index = FT_Get_Char_Index(face, cp);
        if (FT_Load_Glyph(face, index, FT_LOAD_DEFAULT) == 0) {
            width += static_cast<float>(face->glyph->advance.x >> 6);
        }
    }
    chain.dispose();
    return width;
}

bool rasterize_text(const std::vector<TextJob>& jobs, std::vector<UiQuad>& quads,
                    std::vector<uint8_t>& atlas_rgba, uint32_t& atlas_w, uint32_t& atlas_h,
                    std::string& error, std::set<char32_t>* missing_out) {
    atlas_w = 1024;
    atlas_rgba.assign(static_cast<size_t>(atlas_w) * 1024 * 4, 0);
    AtlasPacker packer(atlas_rgba, atlas_w);
    std::map<std::string, FaceChain> chains;
    auto get_chain = [&](const std::string& path) -> FaceChain* {
        auto it = chains.find(path);
        if (it != chains.end()) return &it->second;
        FaceChain chain = load_chain(path);
        if (chain.faces.empty()) {
            if (error.empty()) error = "font unavailable";
            return nullptr;
        }
        auto [inserted, ok] = chains.emplace(path, std::move(chain));
        (void)ok;
        return &inserted->second;
    };

    const size_t first_quad = quads.size();
    for (const TextJob& job : jobs) {
        FaceChain* chain = get_chain(job.font_path);
        if (!chain) continue;
        chain->set_size(job.size_px);
        FT_Face primary = chain->faces.front();
        float pen_x = job.x;
        float baseline = job.y;
        for (char32_t cp : decode_utf8(job.text)) {
            if (cp == U'\n') continue;
            FT_Face face = chain->pick(cp);
            FT_UInt index = 0;
            if (face) {
                index = FT_Get_Char_Index(face, cp);
            } else {
                // No installed font covers this codepoint: draw the
                // primary font's .notdef box and report it (the caller
                // turns this into a scene warning).
                if (missing_out) missing_out->insert(cp);
                face = primary;
                index = 0;
            }
            if (FT_Load_Glyph(face, index, FT_LOAD_RENDER) != 0) continue;
            FT_GlyphSlot glyph = face->glyph;
            float gx = pen_x + glyph->bitmap_left;
            float gy = baseline - glyph->bitmap_top;
            pen_x += static_cast<float>(glyph->advance.x >> 6);
            FT_Bitmap& bmp = glyph->bitmap;
            if (bmp.width == 0 || bmp.rows == 0) continue;
            // Clip: drop glyphs fully outside the clip rect.
            if (gx + bmp.width < job.clip_x || gx > job.clip_x + job.clip_w ||
                gy + bmp.rows < job.clip_y || gy > job.clip_y + job.clip_h) {
                continue;
            }
            // Normalise bitmap pixel modes to 8-bit coverage.
            std::vector<unsigned char> converted;
            const unsigned char* coverage = bmp.buffer;
            uint32_t coverage_pitch = static_cast<uint32_t>(std::abs(bmp.pitch));
            if (bmp.pixel_mode == FT_PIXEL_MODE_MONO) {
                converted.assign(static_cast<size_t>(bmp.width) * bmp.rows, 0);
                for (uint32_t row = 0; row < bmp.rows; ++row) {
                    for (uint32_t col = 0; col < bmp.width; ++col) {
                        unsigned char byte = bmp.buffer[static_cast<size_t>(row) * coverage_pitch + col / 8];
                        converted[static_cast<size_t>(row) * bmp.width + col] =
                            (byte & (0x80 >> (col % 8))) ? 255 : 0;
                    }
                }
                coverage = converted.data();
                coverage_pitch = bmp.width;
            } else if (bmp.pixel_mode == FT_PIXEL_MODE_BGRA) {
                // Colour glyph (e.g. emoji): use its alpha as coverage;
                // the glyph shape is drawn in the text colour (colour
                // bitmap fonts are not reproduced — see COMPATIBILITY.md).
                converted.assign(static_cast<size_t>(bmp.width) * bmp.rows, 0);
                for (size_t px = 0; px < converted.size(); ++px) {
                    converted[px] = bmp.buffer[px * 4 + 3];
                }
                coverage = converted.data();
                coverage_pitch = bmp.width;
            } else if (bmp.pixel_mode != FT_PIXEL_MODE_GRAY) {
                continue;
            }
            auto rect = packer.add(coverage, bmp.width, bmp.rows, coverage_pitch);
            UiQuad quad;
            quad.x = gx;
            quad.y = gy;
            quad.width = static_cast<float>(bmp.width);
            quad.height = static_cast<float>(bmp.rows);
            quad.u0 = rect[0] / atlas_w;
            quad.u1 = rect[2] / atlas_w;
            quad.v0 = rect[1]; // pixel space; normalised below
            quad.v1 = rect[3];
            quad.color = job.color;
            quad.use_atlas = true;
            quad.sort_key = job.sort_key;
            quad.owner_path = job.owner_path;
            quad.sequence = job.sequence;
            quads.push_back(quad);
        }
    }
    // Finalise the atlas: it may have grown past 1024 rows; only now is
    // the true height known, so normalise v coordinates here.
    atlas_h = std::max(1024u, packer.height());
    if (atlas_rgba.size() < static_cast<size_t>(atlas_w) * atlas_h * 4) {
        atlas_rgba.resize(static_cast<size_t>(atlas_w) * atlas_h * 4, 0);
    }
    for (size_t i = first_quad; i < quads.size(); ++i) {
        quads[i].v0 /= atlas_h;
        quads[i].v1 /= atlas_h;
    }
    for (auto& [path, chain] : chains) chain.dispose();
    return true;
}

#else

float measure_text(const std::string& /*text*/, float /*size_px*/, const std::string& /*font_path*/) {
    return 0.0f;
}

bool rasterize_text(const std::vector<TextJob>& /*jobs*/, std::vector<UiQuad>& /*quads*/,
                    std::vector<uint8_t>& /*atlas*/, uint32_t& /*w*/, uint32_t& /*h*/,
                    std::string& error, std::set<char32_t>* /*missing*/) {
    error = "FreeType is not available in this build (libfreetype-dev missing at configure time)";
    return false;
}

#endif

#ifndef BLOX_HAVE_FREETYPE
namespace {
bool is_cjk_codepoint(char32_t cp) {
    return (cp >= 0x2E80 && cp <= 0x9FFF) || (cp >= 0xF900 && cp <= 0xFAFF) ||
           (cp >= 0x20000 && cp <= 0x2FA1F);
}
} // namespace
#endif

// (blox namespace continues; layout engine below)

// ===================== GUI layout engine (Phase 4) =====================
namespace {

const Variant* uprop(const Instance& inst, const std::string& name) {
    auto it = inst.properties.find(name);
    return it == inst.properties.end() ? nullptr : &it->second;
}
double unum(const Variant& v, size_t idx, double fallback = 0) {
    if (v.type == ValueType::UDim2 || v.type == ValueType::UDim || v.type == ValueType::Vector2 ||
        v.type == ValueType::NumberRange || v.type == ValueType::Rect) {
        if (v.complex_value.is_array() && idx < v.complex_value.as_array().size())
            return v.complex_value.as_array()[idx].as_double();
        return fallback;
    }
    return fallback;
}
// Returns [xScale, xOffset, yScale, yOffset].
std::array<double, 4> uudim2(const Instance& inst, const char* name, std::array<double, 4> fallback) {
    const Variant* v = uprop(inst, name);
    if (!v) return fallback;
    return {unum(*v, 0, fallback[0]), unum(*v, 1, fallback[1]), unum(*v, 2, fallback[2]), unum(*v, 3, fallback[3])};
}
std::array<double, 2> uudim(const Instance& inst, const char* name, std::array<double, 2> fallback) {
    const Variant* v = uprop(inst, name);
    if (!v) {
        // UIPadding sides are individual UDim properties.
        return fallback;
    }
    return {unum(*v, 0, fallback[0]), unum(*v, 1, fallback[1])};
}
std::array<double, 2> uvec2(const Instance& inst, const char* name, std::array<double, 2> fallback) {
    const Variant* v = uprop(inst, name);
    if (!v) return fallback;
    if (v->type == ValueType::Vector3) return {v->vec3[0], v->vec3[1]};
    if (v->complex_value.is_array() && v->complex_value.as_array().size() >= 2)
        return {v->complex_value.as_array()[0].as_double(), v->complex_value.as_array()[1].as_double()};
    return fallback;
}
std::array<float, 3> ucolor(const Instance& inst, const char* name, std::array<float, 3> fallback) {
    const Variant* v = uprop(inst, name);
    if (v && v->type == ValueType::Color3)
        return {static_cast<float>(v->vec3[0]), static_cast<float>(v->vec3[1]), static_cast<float>(v->vec3[2])};
    return fallback;
}
double ufloat(const Instance& inst, const char* name, double fallback) {
    const Variant* v = uprop(inst, name);
    if (!v) return fallback;
    if (v->type == ValueType::Float32 || v->type == ValueType::Float64) return v->float_value;
    if (v->type == ValueType::Int32 || v->type == ValueType::Int64) return static_cast<double>(v->int_value);
    return fallback;
}
bool ubool(const Instance& inst, const char* name, bool fallback) {
    const Variant* v = uprop(inst, name);
    return (v && v->type == ValueType::Bool) ? v->bool_value : fallback;
}
int64_t uenum(const Instance& inst, const char* name, int64_t fallback) {
    const Variant* v = uprop(inst, name);
    if (v && (v->type == ValueType::Enum || v->type == ValueType::Int32 || v->type == ValueType::Int64))
        return v->int_value;
    if (v && v->type == ValueType::EnumItem && v->complex_value.is_object()) {
        const Json* val = v->complex_value.find("value");
        if (val) return val->as_int(fallback);
    }
    return fallback;
}
std::string ustring(const Instance& inst, const char* name) {
    const Variant* v = uprop(inst, name);
    if (v && (v->type == ValueType::String || v->type == ValueType::ContentId)) return v->string_value;
    return "";
}

const Instance* decoration(const Instance& inst, const char* class_name) {
    for (const auto& child : inst.children)
        if (child->class_name == class_name) return child.get();
    return nullptr;
}

bool is_gui_object(const std::string& cls) {
    static const std::set<std::string> classes = {
        "Frame", "TextLabel", "TextButton", "TextBox", "ImageLabel", "ImageButton",
        "ScrollingFrame", "CanvasGroup", "GuiObject"};
    return classes.count(cls) > 0;
}
bool is_text_object(const std::string& cls) {
    return cls == "TextLabel" || cls == "TextButton" || cls == "TextBox";
}

// Font file for a text object: FontFace (family/weight) or legacy Font enum.
// Roblox's own fonts (Montserrat, SourceSansPro, ...) are not redistributable
// here; everything maps to the legally distributable DejaVu family and the
// substitution is documented in COMPATIBILITY.md.
std::string font_file_for(const Instance& inst, const UiOptions& options) {
    if (!options.font_path.empty()) return options.font_path;
    int weight = 400;
    if (const Variant* ff = uprop(inst, "FontFace")) {
        if (ff->type == ValueType::Font && ff->complex_value.is_object()) {
            const Json* w = ff->complex_value.find("weight");
            if (w) weight = static_cast<int>(w->as_int(400));
        }
    }
    const char* bold = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf";
    const char* regular = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";
    if (weight >= 600) {
        FILE* f = fopen(bold, "rb");
        if (f) { fclose(f); return bold; }
    }
    return regular;
}

std::string strip_rich_text(const std::string& text) {
    std::string out;
    bool in_tag = false;
    for (char c : text) {
        if (c == '<') in_tag = true;
        else if (c == '>') in_tag = false;
        else if (!in_tag) out.push_back(c);
    }
    return out;
}

// ------------------------------------------------------------- RichText --
// Minimal Roblox RichText subset (Phase 16): <b>, <i>, <u>,
// <font color="#RRGGBB"> / rgb(r,g,b), <br>, and the standard entities.
// Unsupported tags/attributes are stripped (same visible text as
// Roblox, without their styling) — never substituted.
struct RichSpan {
    std::string text;
    bool bold = false;
    bool italic = false;
    bool underline = false;
    bool has_color = false;
    std::array<float, 3> color{0, 0, 0};
};

bool parse_rich_color(const std::string& attr, std::array<float, 3>& out) {
    size_t hash = attr.find('#');
    if (hash != std::string::npos && hash + 7 <= attr.size()) {
        auto hex = [&](size_t at) { return std::stoi(attr.substr(at, 2), nullptr, 16); };
        try {
            out = {hex(hash + 1) / 255.0f, hex(hash + 3) / 255.0f, hex(hash + 5) / 255.0f};
            return true;
        } catch (...) { return false; }
    }
    size_t rgb = attr.find("rgb(");
    if (rgb != std::string::npos) {
        float r = 0, g = 0, b = 0;
        if (std::sscanf(attr.c_str() + rgb, "rgb(%f,%f,%f)", &r, &g, &b) == 3) {
            out = {r / 255.0f, g / 255.0f, b / 255.0f};
            return true;
        }
    }
    return false;
}

std::vector<RichSpan> parse_rich_text(const std::string& raw) {
    std::vector<RichSpan> spans;
    RichSpan cur;
    auto flush = [&]() {
        if (!cur.text.empty()) { spans.push_back(cur); cur.text.clear(); }
    };
    size_t i = 0;
    while (i < raw.size()) {
        if (raw[i] == '<') {
            size_t end = raw.find('>', i);
            if (end == std::string::npos) { cur.text += raw.substr(i); break; }
            std::string tag = raw.substr(i + 1, end - i - 1);
            flush();
            std::string low;
            for (char c : tag) low += static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
            if (low == "b") cur.bold = true;
            else if (low == "/b") cur.bold = false;
            else if (low == "i") cur.italic = true;
            else if (low == "/i") cur.italic = false;
            else if (low == "u") cur.underline = true;
            else if (low == "/u") cur.underline = false;
            else if (low == "br" || low == "br/") cur.text += "\n";
            else if (low.rfind("font", 0) == 0) {
                std::array<float, 3> col;
                if (parse_rich_color(low, col)) { cur.has_color = true; cur.color = col; }
            } else if (low == "/font") {
                cur.has_color = false;
            }
            i = end + 1;
        } else if (raw[i] == '&') {
            if (raw.compare(i, 4, "&lt;") == 0) { cur.text += '<'; i += 4; }
            else if (raw.compare(i, 4, "&gt;") == 0) { cur.text += '>'; i += 4; }
            else if (raw.compare(i, 5, "&amp;") == 0) { cur.text += '&'; i += 5; }
            else if (raw.compare(i, 6, "&quot;") == 0) { cur.text += '"'; i += 6; }
            else { cur.text += raw[i]; ++i; }
        } else {
            cur.text += raw[i];
            ++i;
        }
    }
    flush();
    return spans;
}

std::string rich_font_for(const RichSpan& span, const std::string& base_font) {
    const char* bold = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf";
    const char* oblique = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Oblique.ttf";
    const char* want = span.bold ? bold : (span.italic ? oblique : nullptr);
    if (want) {
        FILE* f = fopen(want, "rb");
        if (f) { fclose(f); return want; }
    }
    return base_font;
}

std::vector<std::string> wrap_lines(const std::string& text, float size_px, const std::string& font,
                                    float max_width, bool wrapped) {
    std::vector<std::string> lines;
    std::string current;
    // Split into words (keeping explicit newlines as breaks).
    std::vector<std::string> tokens;
    std::string word;
    // Byte-wise scan is UTF-8-safe here: ASCII separators never occur
    // inside a multibyte sequence. CJK codepoints additionally become
    // single-character tokens so unspaced CJK text can wrap (UAX #14
    // allows breaks between ideographs).
    for (size_t i = 0; i < text.size();) {
        char c = text[i];
        if (c == '\n') { tokens.push_back(word); tokens.push_back("\n"); word.clear(); ++i; continue; }
        if (c == ' ') { tokens.push_back(word); tokens.push_back(" "); word.clear(); ++i; continue; }
        unsigned char lead = static_cast<unsigned char>(c);
        size_t len = lead < 0x80 ? 1 : (lead >= 0xF0 ? 4 : (lead >= 0xE0 ? 3 : (lead >= 0xC0 ? 2 : 1)));
        if (len > 1 && i + len <= text.size()) {
            // Decode just enough to spot CJK ranges.
            char32_t cp = lead & (len == 2 ? 0x1F : len == 3 ? 0x0F : 0x07);
            for (size_t k = 1; k < len; ++k) cp = (cp << 6) | (static_cast<unsigned char>(text[i + k]) & 0x3F);
            if (is_cjk_codepoint(cp)) {
                tokens.push_back(word);
                word.clear();
                tokens.push_back(text.substr(i, len));
                i += len;
                continue;
            }
        }
        word += text.substr(i, len);
        i += len;
    }
    tokens.push_back(word);
    for (const auto& token : tokens) {
        if (token == "\n") { lines.push_back(current); current.clear(); continue; }
        if (!wrapped) { current += token; continue; }
        std::string candidate = current + token;
        if (measure_text(candidate, size_px, font) > max_width && !current.empty()) {
            lines.push_back(current);
            current = token == " " ? "" : token;
        } else {
            current = candidate;
        }
    }
    lines.push_back(current);
    return lines;
}

struct RectF { float x, y, w, h; };
RectF intersect(RectF a, RectF b) {
    float x = std::max(a.x, b.x), y = std::max(a.y, b.y);
    float w = std::min(a.x + a.w, b.x + b.w) - x;
    float h = std::min(a.y + a.h, b.y + b.h) - y;
    return {x, y, std::max(w, 0.0f), std::max(h, 0.0f)};
}

struct LayoutCtx {
    const UiOptions& options;
    UiScene& scene;
    uint32_t sequence = 0;
    std::vector<TextJob>& text_jobs;
    explicit LayoutCtx(const UiOptions& o, UiScene& s, std::vector<TextJob>& jobs)
        : options(o), scene(s), text_jobs(jobs) {}
};

struct ChildResult {
    const Instance* inst;
    RectF rect;
};

RectF layout_object(const Instance& inst, RectF parent_content, float inherited_scale,
                    RectF clip, float alpha_acc, bool parent_visible,
                    const std::vector<int>& zkey_prefix, bool emit,
                    const std::string& parent_path, LayoutCtx& ctx,
                    const RectF* forced_rect = nullptr);

void add_quad(LayoutCtx& ctx, UiQuad& quad, const RectF& clip, const std::vector<int>& zkey) {
    quad.clip_x = clip.x; quad.clip_y = clip.y; quad.clip_w = clip.w; quad.clip_h = clip.h;
    quad.sort_key = zkey;
    quad.sequence = ctx.sequence++;
    ctx.scene.quads.push_back(quad);
}

// Emits background/stroke/image/text for one laid-out object (not children).
void emit_own_quads(const Instance& inst, const RectF& rect, const RectF& content,
                    RectF clip, float alpha_acc, const std::vector<int>& zkey,
                    const std::string& path, LayoutCtx& ctx) {
    const std::string& cls = inst.class_name;
    float own_alpha = alpha_acc;
    if (cls == "CanvasGroup") own_alpha *= 1.0f - static_cast<float>(ufloat(inst, "GroupTransparency", 0.0));
    const Instance* corner = decoration(inst, "UICorner");
    float radius = 0;
    if (corner) {
        auto cr = uudim(*corner, "CornerRadius", {0, 0});
        radius = static_cast<float>(cr[0] * std::min(rect.w, rect.h) + cr[1]);
    }
    const Instance* stroke = decoration(inst, "UIStroke");
    if (stroke && ubool(*stroke, "Enabled", true)) {
        float thickness = static_cast<float>(ufloat(*stroke, "Thickness", 1.0));
        auto sc = ucolor(*stroke, "Color", {0, 0, 0});
        float sa = (1.0f - static_cast<float>(ufloat(*stroke, "Transparency", 0.0))) * own_alpha;
        if (sa > 0.003f && thickness > 0) {
            UiQuad q;
            q.x = rect.x - thickness; q.y = rect.y - thickness;
            q.width = rect.w + thickness * 2; q.height = rect.h + thickness * 2;
            q.color = {sc[0], sc[1], sc[2], sa};
            q.corner_radius = radius + thickness;
            q.owner_path = path;
            add_quad(ctx, q, clip, zkey);
        }
    }
    // Background.
    float bg_transparency = static_cast<float>(ufloat(inst, "BackgroundTransparency", 0.0));
    if (cls != "ScreenGui" && bg_transparency < 1.0f) {
        auto bg = ucolor(inst, "BackgroundColor3", {1, 1, 1});
        float a = (1.0f - bg_transparency) * own_alpha;
        if (a > 0.003f) {
            UiQuad q;
            q.x = rect.x; q.y = rect.y; q.width = rect.w; q.height = rect.h;
            q.color = {bg[0], bg[1], bg[2], a};
            q.corner_radius = radius;
            q.owner_path = path;
            if (const Instance* grad = decoration(inst, "UIGradient")) {
                if (ubool(*grad, "Enabled", true)) {
                    q.has_gradient = true;
                    q.gradient_degrees = static_cast<float>(ufloat(*grad, "Rotation", 0.0));
                    // Last keypoint colour of the Color sequence, tinted by bg.
                    std::array<float, 3> endc = bg;
                    if (const Variant* seq = uprop(*grad, "Color")) {
                        if (seq->complex_value.is_array() && !seq->complex_value.as_array().empty()) {
                            const Json& last = seq->complex_value.as_array().back();
                            if (const Json* col = last.find("color")) {
                                if (col->is_array() && col->as_array().size() == 3) {
                                    endc = {bg[0] * static_cast<float>(col->as_array()[0].as_double()),
                                            bg[1] * static_cast<float>(col->as_array()[1].as_double()),
                                            bg[2] * static_cast<float>(col->as_array()[2].as_double())};
                                }
                            }
                        }
                    }
                    float end_alpha = a;
                    if (const Variant* tseq = uprop(*grad, "Transparency")) {
                        if (tseq->complex_value.is_array() && !tseq->complex_value.as_array().empty()) {
                            const Json& last = tseq->complex_value.as_array().back();
                            if (const Json* val = last.find("value"))
                                end_alpha = (1.0f - static_cast<float>(val->as_double())) * own_alpha;
                        }
                    }
                    q.gradient_end = {endc[0], endc[1], endc[2], end_alpha};
                }
            }
            add_quad(ctx, q, clip, zkey);
        }
    }
    // Image.
    if (cls == "ImageLabel" || cls == "ImageButton") {
        std::string image = ustring(inst, "Image");
        auto tint = ucolor(inst, "ImageColor3", {1, 1, 1});
        float a = (1.0f - static_cast<float>(ufloat(inst, "ImageTransparency", 0.0))) * own_alpha;
        if (!image.empty() && a > 0.003f) {
            bool found = false;
            // Resolve via the same asset rules as meshes (PNG only).
            std::string resolved;
            {
                // Local re-implementation of asset search (assets/place dirs).
                std::string id = image;
                const std::string p1 = "rbxassetid://";
                if (id.rfind(p1, 0) == 0) id = id.substr(p1.size());
                bool numeric = !id.empty() && std::all_of(id.begin(), id.end(), [](char c) { return c >= '0' && c <= '9'; });
                std::vector<std::string> candidates;
                if (numeric) {
                    if (!ctx.options.assets_dir.empty()) candidates.push_back(ctx.options.assets_dir + "/" + id + ".png");
                } else {
                    size_t slash = id.find_last_of("/\\");
                    std::string base = slash == std::string::npos ? id : id.substr(slash + 1);
                    if (!ctx.options.assets_dir.empty()) candidates.push_back(ctx.options.assets_dir + "/" + base);
                    if (!ctx.options.place_dir.empty()) candidates.push_back(ctx.options.place_dir + "/" + base);
                    candidates.push_back(id);
                }
                for (const auto& cand : candidates) {
                    FILE* f = fopen(cand.c_str(), "rb");
                    if (f) { fclose(f); resolved = cand; found = true; break; }
                }
            }
            if (found) {
                UiQuad q;
                q.x = rect.x; q.y = rect.y; q.width = rect.w; q.height = rect.h;
                q.color = {tint[0], tint[1], tint[2], a};
                q.texture_path = resolved;
                q.corner_radius = radius;
                q.owner_path = path;
                int64_t scale_type = uenum(inst, "ScaleType", 0);
                bool sliced = false;
                if (scale_type == 1) { // Slice (nine-slice, Phase 16)
                    auto center = uudim2(inst, "SliceCenter", {0, 0, 0, 0});
                    float slice_scale = static_cast<float>(ufloat(inst, "SliceScale", 1.0));
                    // PNG dimensions from the IHDR header.
                    uint32_t iw = 0, ih = 0;
                    if (FILE* pf = fopen(resolved.c_str(), "rb")) {
                        uint8_t hdr[24]{};
                        if (std::fread(hdr, 1, 24, pf) == 24 && hdr[1] == 'P' && hdr[2] == 'N' && hdr[3] == 'G') {
                            iw = (static_cast<uint32_t>(hdr[16]) << 24) | (hdr[17] << 16) | (hdr[18] << 8) | hdr[19];
                            ih = (static_cast<uint32_t>(hdr[20]) << 24) | (hdr[21] << 16) | (hdr[22] << 8) | hdr[23];
                        }
                        fclose(pf);
                    }
                    if (iw > 0 && ih > 0 && center[2] > center[0] && center[3] > center[1]) {
                        float sx[4] = {0, static_cast<float>(center[0]), static_cast<float>(center[2]), static_cast<float>(iw)};
                        float sy[4] = {0, static_cast<float>(center[1]), static_cast<float>(center[3]), static_cast<float>(ih)};
                        float bl = sx[1] * slice_scale, br = (iw - sx[2]) * slice_scale;
                        float bt = sy[1] * slice_scale, bb = (ih - sy[2]) * slice_scale;
                        float dx[4] = {rect.x, rect.x + bl, rect.x + rect.w - br, rect.x + rect.w};
                        float dy[4] = {rect.y, rect.y + bt, rect.y + rect.h - bb, rect.y + rect.h};
                        for (int row = 0; row < 3; ++row) {
                            for (int col = 0; col < 3; ++col) {
                                UiQuad cell = q;
                                cell.x = dx[col]; cell.y = dy[row];
                                cell.width = dx[col + 1] - dx[col];
                                cell.height = dy[row + 1] - dy[row];
                                cell.u0 = sx[col] / iw; cell.u1 = sx[col + 1] / iw;
                                cell.v0 = sy[row] / ih; cell.v1 = sy[row + 1] / ih;
                                if (cell.width > 0 && cell.height > 0) add_quad(ctx, cell, clip, zkey);
                            }
                        }
                        sliced = true;
                    }
                    ctx.scene.warnings.push_back(path + ": SliceCenter/texture size unavailable; rendered as stretch");
                }
                if (!sliced) {
                auto rect_offset = uvec2(inst, "ImageRectOffset", {0, 0});
                auto rect_size = uvec2(inst, "ImageRectSize", {0, 0});
                if (scale_type == 2) { // Tile
                    auto tile = uudim2(inst, "TileSize", {0, 32, 0, 32});
                    float tw = std::max(static_cast<float>(tile[0] * rect.w + tile[1]), 1.0f);
                    float th = std::max(static_cast<float>(tile[2] * rect.h + tile[3]), 1.0f);
                    q.u1 = rect.w / tw;
                    q.v1 = rect.h / th;
                } else if (rect_size[0] > 0 && rect_size[1] > 0) {
                    ctx.scene.warnings.push_back(path + ": ImageRectSize sub-rectangle used without a known texture size; rendered as full-texture stretch");
                }
                add_quad(ctx, q, clip, zkey);
                }
            } else {
                ctx.scene.warnings.push_back(path + ": Image '" + image + "' not found locally; image not drawn (not substituted)");
            }
        }
    }
    // Text.
    if (is_text_object(cls)) {
        std::string raw_text = ustring(inst, "Text");
        std::string text = raw_text;
        if (ubool(inst, "RichText", false)) text = strip_rich_text(text);
        if (text.empty()) return;
        std::string font = font_file_for(inst, ctx.options);
        float text_size = static_cast<float>(ufloat(inst, "TextSize", 14.0));
        bool scaled = ubool(inst, "TextScaled", false);
        bool wrapped = ubool(inst, "TextWrapped", false);
        const Instance* tconstraint = decoration(inst, "UITextSizeConstraint");
        float min_size = tconstraint ? static_cast<float>(ufloat(*tconstraint, "MinTextSize", 1.0)) : 1.0f;
        float max_size = tconstraint ? static_cast<float>(ufloat(*tconstraint, "MaxTextSize", 100.0)) : 100.0f;
        float size = text_size;
        if (scaled) {
            // Largest size (binary search) whose wrapped text fits the box.
            float lo = std::max(min_size, 1.0f), hi = std::max(max_size, lo), best = lo;
            for (int iter = 0; iter < 12; ++iter) {
                float mid = (lo + hi) / 2;
                auto lines = wrap_lines(text, mid, font, content.w, true);
                float total_h = static_cast<float>(lines.size()) * mid * 1.25f;
                bool fits = total_h <= content.h;
                for (const auto& line : lines) fits = fits && measure_text(line, mid, font) <= content.w;
                if (fits) { best = mid; lo = mid; } else { hi = mid; }
            }
            size = best;
        }
        auto color = ucolor(inst, "TextColor3", {0, 0, 0});
        float a = (1.0f - static_cast<float>(ufloat(inst, "TextTransparency", 0.0))) * own_alpha;
        if (a <= 0.003f) return;
        // RichText layout (Phase 16): styled spans wrapped by word,
        // one TextJob per style run; underlines are thin quads.
        if (ubool(inst, "RichText", false) && raw_text.find('<') != std::string::npos) {
            auto spans = parse_rich_text(raw_text);
            struct RRun { std::string text; RichSpan style; float width = 0; };
            auto rich_wrap = [&](float sz) {
                struct Tok { std::string word; RichSpan style; bool space_before; bool break_after; };
                std::vector<Tok> toks;
                bool carry_space = false;
                for (const auto& span : spans) {
                    std::string cur_word;
                    bool space_before = carry_space;
                    auto push_word = [&]() {
                        if (!cur_word.empty()) {
                            toks.push_back({cur_word, span, space_before, false});
                            cur_word.clear();
                            space_before = false;
                        }
                    };
                    for (char c : span.text) {
                        if (c == '\n') { push_word(); toks.push_back({"", span, false, true}); space_before = false; }
                        else if (c == ' ') { push_word(); space_before = true; }
                        else cur_word += c;
                    }
                    push_word();
                    carry_space = space_before;
                }
                std::vector<std::vector<RRun>> lines;
                std::vector<RRun> line;
                float line_w = 0;
                auto flush_line = [&]() { lines.push_back(line); line.clear(); line_w = 0; };
                for (const auto& tok : toks) {
                    if (tok.break_after) { flush_line(); continue; }
                    float space_w = (tok.space_before && !line.empty())
                        ? measure_text(" ", sz, rich_font_for(tok.style, font)) : 0;
                    float word_w = measure_text(tok.word, sz, rich_font_for(tok.style, font));
                    if ((wrapped || scaled) && !line.empty() && line_w + space_w + word_w > content.w) {
                        flush_line();
                        space_w = 0;
                    }
                    std::string piece = (space_w > 0 ? " " : "") + tok.word;
                    float piece_w = space_w + word_w;
                    if (!line.empty() && line.back().style.bold == tok.style.bold &&
                        line.back().style.italic == tok.style.italic &&
                        line.back().style.underline == tok.style.underline &&
                        line.back().style.has_color == tok.style.has_color &&
                        line.back().style.color == tok.style.color) {
                        line.back().text += piece;
                        line.back().width += piece_w;
                    } else {
                        line.push_back({piece, tok.style, piece_w});
                    }
                    line_w += piece_w;
                }
                flush_line();
                return lines;
            };
            float rsize = size;
            if (scaled) {
                float lo = std::max(min_size, 1.0f), hi = std::max(max_size, lo), best = lo;
                for (int iter = 0; iter < 12; ++iter) {
                    float mid = (lo + hi) / 2;
                    auto lines = rich_wrap(mid);
                    bool fits = static_cast<float>(lines.size()) * mid * 1.25f <= content.h;
                    for (const auto& line : lines) {
                        float w = 0;
                        for (const auto& run : line) w += run.width;
                        fits = fits && w <= content.w;
                    }
                    if (fits) { best = mid; lo = mid; } else { hi = mid; }
                }
                rsize = best;
            }
            auto rich_lines = rich_wrap(rsize);
            int64_t ryalign = uenum(inst, "TextYAlignment", 1);
            int64_t rxalign = uenum(inst, "TextXAlignment", 0);
            float rline_h = rsize * 1.25f;
            float rtotal_h = rline_h * static_cast<float>(rich_lines.size());
            float rstart_y = content.y;
            if (ryalign == 1) rstart_y = content.y + (content.h - rtotal_h) / 2;
            else if (ryalign == 2) rstart_y = content.y + (content.h - rtotal_h);
            for (size_t i = 0; i < rich_lines.size(); ++i) {
                float line_w = 0;
                for (const auto& run : rich_lines[i]) line_w += run.width;
                float lx = content.x;
                if (rxalign == 1) lx = content.x + (content.w - line_w) / 2;
                else if (rxalign == 2) lx = content.x + (content.w - line_w);
                float cx = lx;
                for (const auto& run : rich_lines[i]) {
                    if (run.text.empty()) { continue; }
                    TextJob job;
                    job.text = run.text;
                    job.x = cx;
                    job.y = rstart_y + static_cast<float>(i) * rline_h + rsize;
                    job.size_px = rsize;
                    auto rc = run.style.has_color ? run.style.color : color;
                    job.color = {rc[0], rc[1], rc[2], a};
                    job.font_path = rich_font_for(run.style, font);
                    job.clip_x = clip.x; job.clip_y = clip.y; job.clip_w = clip.w; job.clip_h = clip.h;
                    job.sort_key = zkey;
                    job.owner_path = path;
                    job.sequence = ctx.sequence++;
                    ctx.text_jobs.push_back(job);
                    if (run.style.underline) {
                        UiQuad underline;
                        underline.x = cx;
                        underline.y = rstart_y + static_cast<float>(i) * rline_h + rsize * 1.12f;
                        underline.width = run.width;
                        underline.height = std::max(1.0f, rsize / 14.0f);
                        underline.color = {rc[0], rc[1], rc[2], a};
                        underline.owner_path = path;
                        add_quad(ctx, underline, clip, zkey);
                    }
                    cx += run.width;
                }
            }
            return;
        }
        auto lines = wrap_lines(text, size, font, content.w, wrapped || scaled);
        // Truncation (AtEnd) for a single unwrapped overflowing line.
        if (uenum(inst, "TextTruncate", 0) == 1 && !wrapped && lines.size() == 1) {
            std::string line = lines[0];
            auto pop_codepoint = [](std::string& str) {
                while (!str.empty() && (static_cast<unsigned char>(str.back()) & 0xC0) == 0x80) str.pop_back();
                if (!str.empty()) str.pop_back();
            };
            while (!line.empty() && measure_text(line + "...", size, font) > content.w) pop_codepoint(line);
            if (measure_text(line + "...", size, font) <= content.w) lines[0] = line + "...";
        }
        float line_h = size * 1.25f;
        float total_h = line_h * static_cast<float>(lines.size());
        int64_t yalign = uenum(inst, "TextYAlignment", 1); // Top=0 Center=1 Bottom=2 (Centre default in Roblox is Centre)
        float start_y = content.y;
        if (yalign == 1) start_y = content.y + (content.h - total_h) / 2;
        else if (yalign == 2) start_y = content.y + (content.h - total_h);
        int64_t xalign = uenum(inst, "TextXAlignment", 0); // Left=0 Centre=1 Right=2
        for (size_t i = 0; i < lines.size(); ++i) {
            float line_w = measure_text(lines[i], size, font);
            float lx = content.x;
            if (xalign == 1) lx = content.x + (content.w - line_w) / 2;
            else if (xalign == 2) lx = content.x + (content.w - line_w);
            TextJob job;
            job.text = lines[i];
            job.x = lx;
            job.y = start_y + static_cast<float>(i) * line_h + size; // baseline approx
            job.size_px = size;
            job.color = {color[0], color[1], color[2], a};
            job.font_path = font;
            job.clip_x = clip.x; job.clip_y = clip.y; job.clip_w = clip.w; job.clip_h = clip.h;
            job.sort_key = zkey;
            job.owner_path = path;
            job.sequence = ctx.sequence++;
            ctx.text_jobs.push_back(job);
        }
    }
}

} // namespace
namespace {

RectF layout_object(const Instance& inst, RectF parent_content, float inherited_scale,
                    RectF clip, float alpha_acc, bool parent_visible,
                    const std::vector<int>& zkey_prefix, bool emit,
                    const std::string& parent_path, LayoutCtx& ctx,
                    const RectF* forced_rect) {
    const std::string& cls = inst.class_name;
    std::string path = parent_path.empty() ? inst.name : parent_path + "/" + inst.name;
    bool visible = parent_visible && ubool(inst, "Visible", true);

    const Instance* ui_scale = decoration(inst, "UIScale");
    float own_scale = inherited_scale * (ui_scale ? static_cast<float>(ufloat(*ui_scale, "Scale", 1.0)) : 1.0f);

    RectF rect;
    float anchor_point_x = 0, anchor_point_y = 0, anchor_frac_x = 0, anchor_frac_y = 0;
    if (forced_rect) {
        rect = *forced_rect;
    } else {
        auto size_spec = uudim2(inst, "Size", {0, 100, 0, 100});
        float w = parent_content.w * static_cast<float>(size_spec[0]) + static_cast<float>(size_spec[1]) * own_scale;
        float h = parent_content.h * static_cast<float>(size_spec[2]) + static_cast<float>(size_spec[3]) * own_scale;
        // Size constraints first pass.
        if (const Instance* sc = decoration(inst, "UISizeConstraint")) {
            auto mn = uvec2(*sc, "MinSize", {0, 0});
            auto mx = uvec2(*sc, "MaxSize", {1e9, 1e9});
            w = std::clamp(w, static_cast<float>(mn[0]), static_cast<float>(mx[0]));
            h = std::clamp(h, static_cast<float>(mn[1]), static_cast<float>(mx[1]));
        }
        if (const Instance* ar = decoration(inst, "UIAspectRatioConstraint")) {
            double ratio = ufloat(*ar, "AspectRatio", 1.0);
            if (ratio > 0) {
                int64_t dominant = uenum(*ar, "DominantAxis", 0);
                if (dominant == 1) w = h * static_cast<float>(ratio);
                else h = w / static_cast<float>(ratio);
            }
        }
        auto pos_spec = uudim2(inst, "Position", {0, 0, 0, 0});
        auto anchor = uvec2(inst, "AnchorPoint", {0, 0});
        float anchor_x = parent_content.x + parent_content.w * static_cast<float>(pos_spec[0]) +
                         static_cast<float>(pos_spec[1]) * own_scale;
        float anchor_y = parent_content.y + parent_content.h * static_cast<float>(pos_spec[2]) +
                         static_cast<float>(pos_spec[3]) * own_scale;
        rect = {anchor_x - static_cast<float>(anchor[0]) * w,
                anchor_y - static_cast<float>(anchor[1]) * h, w, h};
        anchor_point_x = anchor_x;
        anchor_point_y = anchor_y;
        anchor_frac_x = static_cast<float>(anchor[0]);
        anchor_frac_y = static_cast<float>(anchor[1]);
    }

    if (std::abs(ufloat(inst, "Rotation", 0.0)) > 0.001 && emit) {
        ctx.scene.warnings.push_back(path + ": Rotation is not rendered in Phase 4 (object drawn unrotated)");
    }

    // Content area after UIPadding.
    RectF content = rect;
    if (const Instance* padding = decoration(inst, "UIPadding")) {
        auto pl = uudim(*padding, "PaddingLeft", {0, 0});
        auto pr = uudim(*padding, "PaddingRight", {0, 0});
        auto pt = uudim(*padding, "PaddingTop", {0, 0});
        auto pb = uudim(*padding, "PaddingBottom", {0, 0});
        float lx = static_cast<float>(pl[0] * rect.w + pl[1] * own_scale);
        float rx = static_cast<float>(pr[0] * rect.w + pr[1] * own_scale);
        float ty = static_cast<float>(pt[0] * rect.h + pt[1] * own_scale);
        float by = static_cast<float>(pb[0] * rect.h + pb[1] * own_scale);
        content = {rect.x + lx, rect.y + ty, std::max(rect.w - lx - rx, 0.0f), std::max(rect.h - ty - by, 0.0f)};
    }

    // AutomaticSize: measure GUI children and grow the box to fit them.
    int64_t automatic = uenum(inst, "AutomaticSize", 0); // None=0 X=1 Y=2 XY=3
    if (!forced_rect && automatic != 0 && visible) {
        bool any = false;
        float extent_w = 0, extent_h = 0;
        for (const auto& child : inst.children) {
            if (!is_gui_object(child->class_name)) continue;
            RectF measured = layout_object(*child, content, own_scale, clip, alpha_acc,
                                           visible, zkey_prefix, false, path, ctx, nullptr);
            extent_w = std::max(extent_w, (measured.x + measured.w) - content.x);
            extent_h = std::max(extent_h, (measured.y + measured.h) - content.y);
            any = true;
        }
        if (is_text_object(cls) && !any) {
            std::string font = font_file_for(inst, ctx.options);
            std::string text = ustring(inst, "Text");
            float size = static_cast<float>(ufloat(inst, "TextSize", 14.0));
            auto lines = wrap_lines(text, size, font, content.w, ubool(inst, "TextWrapped", false));
            for (const auto& line : lines) extent_w = std::max(extent_w, measure_text(line, size, font));
            extent_h = static_cast<float>(lines.size()) * size * 1.25f;
            any = true;
        }
        if (any) {
            float pad_left = content.x - rect.x;
            float pad_right = (rect.x + rect.w) - (content.x + content.w);
            float pad_top = content.y - rect.y;
            float pad_bottom = (rect.y + rect.h) - (content.y + content.h);
            if (automatic == 1 || automatic == 3) rect.w = pad_left + extent_w + pad_right;
            if (automatic == 2 || automatic == 3) rect.h = pad_top + extent_h + pad_bottom;
            rect.x = anchor_point_x - anchor_frac_x * rect.w;
            rect.y = anchor_point_y - anchor_frac_y * rect.h;
            content = {rect.x + pad_left, rect.y + pad_top,
                       std::max(rect.w - pad_left - pad_right, 0.0f),
                       std::max(rect.h - pad_top - pad_bottom, 0.0f)};
        }
    }

    int64_t zindex = uenum(inst, "ZIndex", 1);
    std::vector<int> zkey = zkey_prefix;
    zkey.push_back(static_cast<int>(zindex));

    if (emit) {
        UiRect rec;
        rec.path = path;
        rec.class_name = cls;
        rec.name = inst.name;
        rec.x = rect.x; rec.y = rect.y; rec.width = rect.w; rec.height = rect.h;
        rec.zindex = static_cast<float>(zindex);
        rec.visible = visible;
        rec.clips_descendants = ubool(inst, "ClipsDescendants", cls == "ScrollingFrame");
        rec.parent_path = parent_path;
        if (is_text_object(cls)) rec.text = ustring(inst, "Text");
        ctx.scene.rects.push_back(rec);
        if (visible) {
            RectF own_clip = clip;
            emit_own_quads(inst, rect, content, own_clip, alpha_acc, zkey, path, ctx);
        }
    }

    if (!visible && emit) {
        // Invisible subtrees still get layout records for inspection.
        for (const auto& child : inst.children) {
            if (is_gui_object(child->class_name)) {
                layout_object(*child, content, own_scale, clip, alpha_acc, false, zkey, true, path, ctx, nullptr);
            }
        }
        return rect;
    }
    if (!visible) {
        for (const auto& child : inst.children) {
            if (is_gui_object(child->class_name)) {
                layout_object(*child, content, own_scale, clip, alpha_acc, false, zkey, false, path, ctx, nullptr);
            }
        }
        return rect;
    }

    // Child clip.
    RectF child_clip = clip;
    if (ubool(inst, "ClipsDescendants", cls == "ScrollingFrame" || cls == "CanvasGroup")) {
        child_clip = intersect(clip, content);
    }
    float child_alpha = alpha_acc;
    if (cls == "CanvasGroup") child_alpha *= 1.0f - static_cast<float>(ufloat(inst, "GroupTransparency", 0.0));

    // Collect GUI children (excluding decorations, which are not GuiObjects).
    std::vector<const Instance*> children;
    for (const auto& child : inst.children) {
        if (is_gui_object(child->class_name)) children.push_back(child.get());
        else if (child->class_name == "ViewportFrame" || child->class_name == "VideoFrame" ||
                 child->class_name == "SurfaceGui" || child->class_name == "BillboardGui") {
            if (emit) ctx.scene.warnings.push_back(path + "/" + child->name + ": " + child->class_name +
                                                   " content is rendered by the 3D pass or is unsupported in the 2D UI pass");
        }
    }
    std::stable_sort(children.begin(), children.end(), [](const Instance* a, const Instance* b) {
        return ufloat(*a, "LayoutOrder", 0) < ufloat(*b, "LayoutOrder", 0);
    });

    const Instance* list_layout = decoration(inst, "UIListLayout");
    const Instance* grid_layout = decoration(inst, "UIGridLayout");
    const Instance* page_layout = decoration(inst, "UIPageLayout");

    // ScrollingFrame: shift children by CanvasPosition and clip to content.
    RectF child_area = content;
    if (cls == "ScrollingFrame") {
        auto canvas = uvec2(inst, "CanvasPosition", {0, 0});
        child_area.x -= static_cast<float>(canvas[0]);
        child_area.y -= static_cast<float>(canvas[1]);
        child_clip = intersect(clip, content);
    }

    if (list_layout && !children.empty()) {
        // Measure pass.
        std::vector<RectF> sizes;
        for (const Instance* child : children) {
            RectF measured = layout_object(*child, child_area, own_scale, child_clip, child_alpha,
                                           visible, zkey, false, path, ctx, nullptr);
            sizes.push_back(measured);
        }
        int64_t fill_dir = uenum(*list_layout, "FillDirection", 0); // Vertical=0 Horizontal=1
        auto padding = uudim(*list_layout, "Padding", {0, 0});
        bool horizontal = fill_dir == 1;
        float pad_px = static_cast<float>(horizontal ? padding[0] * content.w + padding[1]
                                                     : padding[0] * content.h + padding[1]);
        float total = 0;
        for (size_t i = 0; i < sizes.size(); ++i) {
            total += horizontal ? sizes[i].w : sizes[i].h;
            if (i) total += pad_px;
        }
        // AutomaticSize on the container itself is handled by the caller via
        // returned rect only for non-forced objects; here we align children.
        int64_t halign = uenum(*list_layout, "HorizontalAlignment", 0);
        int64_t valign = uenum(*list_layout, "VerticalAlignment", 0);
        float main_start;
        float cross_size = horizontal ? content.h : content.w;
        (void)cross_size;
        if (horizontal) {
            main_start = halign == 1 ? content.x + (content.w - total) / 2
                         : halign == 2 ? content.x + (content.w - total) : content.x;
        } else {
            main_start = valign == 1 ? content.y + (content.h - total) / 2
                         : valign == 2 ? content.y + (content.h - total) : content.y;
        }
        float cursor = main_start;
        for (size_t i = 0; i < children.size(); ++i) {
            RectF forced = sizes[i];
            if (horizontal) {
                forced.x = cursor;
                forced.y = valign == 1 ? content.y + (content.h - forced.h) / 2
                           : valign == 2 ? content.y + (content.h - forced.h) : content.y;
                cursor += forced.w + pad_px;
            } else {
                forced.y = cursor;
                forced.x = halign == 1 ? content.x + (content.w - forced.w) / 2
                           : halign == 2 ? content.x + (content.w - forced.w) : content.x;
                cursor += forced.h + pad_px;
            }
            if (emit || true) {
                layout_object(*children[i], child_area, own_scale, child_clip, child_alpha,
                              visible, zkey, emit, path, ctx, &forced);
            }
        }
        return rect;
    }

    if (grid_layout && !children.empty()) {
        auto cell = uudim2(*grid_layout, "CellSize", {0, 100, 0, 100});
        auto cell_pad = uudim2(*grid_layout, "CellPadding", {0, 0, 0, 0});
        float cell_w = content.w * static_cast<float>(cell[0]) + static_cast<float>(cell[1]);
        float cell_h = content.h * static_cast<float>(cell[2]) + static_cast<float>(cell[3]);
        float pad_x = content.w * static_cast<float>(cell_pad[0]) + static_cast<float>(cell_pad[1]);
        float pad_y = content.h * static_cast<float>(cell_pad[2]) + static_cast<float>(cell_pad[3]);
        int64_t max_cells = uenum(*grid_layout, "FillDirectionMaxCells", 0);
        int cols = max_cells > 0 ? static_cast<int>(max_cells)
                                 : std::max(1, static_cast<int>((content.w + pad_x) / std::max(cell_w + pad_x, 1.0f)));
        int64_t fill_dir = uenum(*grid_layout, "FillDirection", 0); // Vertical=0 Horizontal=1
        int rows_per_col = std::max(1, (static_cast<int>(children.size()) + cols - 1) / cols);
        for (size_t i = 0; i < children.size(); ++i) {
            int col, row;
            if (fill_dir == 1) { // Horizontal: row-major
                col = static_cast<int>(i) % cols;
                row = static_cast<int>(i) / cols;
            } else { // Vertical: column-major
                col = static_cast<int>(i) / rows_per_col;
                row = static_cast<int>(i) % rows_per_col;
            }
            RectF cell_rect{content.x + col * (cell_w + pad_x), content.y + row * (cell_h + pad_y), cell_w, cell_h};
            layout_object(*children[i], cell_rect, own_scale, child_clip, child_alpha,
                          visible, zkey, emit, path, ctx, &cell_rect);
        }
        return rect;
    }

    if (page_layout && !children.empty()) {
        for (const Instance* child : children) {
            layout_object(*child, content, own_scale, child_clip, child_alpha,
                          visible, zkey, emit, path, ctx, &content);
        }
        return rect;
    }

    for (const Instance* child : children) {
        layout_object(*child, child_area, own_scale, child_clip, child_alpha,
                      visible, zkey, emit, path, ctx, nullptr);
    }
    return rect;
}

const Instance* find_service(const DataModel& model, const char* class_name) {
    for (const auto& child : model.root().children)
        if (child->class_name == class_name) return child.get();
    return nullptr;
}

// Resolves an engine child-index path ("0/2/1") to an instance.
const Instance* resolve_index_path(const Instance& root, const std::string& index_path) {
    if (index_path.empty()) return &root;
    const Instance* current = &root;
    size_t start = 0;
    while (start <= index_path.size()) {
        size_t slash = index_path.find('/', start);
        std::string part = index_path.substr(start, slash == std::string::npos ? slash : slash - start);
        if (part.empty()) break;
        size_t idx = static_cast<size_t>(std::strtoul(part.c_str(), nullptr, 10));
        if (idx >= current->children.size()) return nullptr;
        current = current->children[idx].get();
        if (slash == std::string::npos) break;
        start = slash + 1;
    }
    return current;
}

} // namespace
namespace {

void finish_ui_scene(LayoutCtx& ctx, UiScene& scene) {
    // Rasterize collected text jobs into the glyph atlas + quads.
    if (!ctx.text_jobs.empty()) {
        std::string error;
        std::vector<UiQuad> glyph_quads;
        std::set<char32_t> missing;
        if (rasterize_text(ctx.text_jobs, glyph_quads, scene.atlas_rgba, scene.atlas_w, scene.atlas_h, error, &missing)) {
            scene.quads.insert(scene.quads.end(), glyph_quads.begin(), glyph_quads.end());
            if (!missing.empty()) {
                std::string list;
                size_t shown = 0;
                for (char32_t cp : missing) {
                    if (shown++ >= 6) { list += ", ..."; break; }
                    char buf[16];
                    std::snprintf(buf, sizeof buf, "%sU+%04X", list.empty() ? "" : ", ", static_cast<unsigned>(cp));
                    list += buf;
                }
                scene.warnings.push_back(
                    "text: no installed font covers " + std::to_string(missing.size()) +
                    " codepoint(s) (" + list + "); drawn as .notdef boxes — install a broader font (e.g. Noto Sans CJK) for full coverage");
            }
        } else if (!error.empty()) {
            scene.warnings.push_back("text: " + error);
        }
    }
    // Stable render order: sort key (ZIndex chain / DisplayOrder) then sequence.
    std::stable_sort(scene.quads.begin(), scene.quads.end(), [](const UiQuad& a, const UiQuad& b) {
        if (a.sort_key != b.sort_key) return a.sort_key < b.sort_key;
        return a.sequence < b.sequence;
    });
    // Record render order on rects by owner path (last quad wins the slot).
    std::map<std::string, int> order_by_path;
    for (size_t i = 0; i < scene.quads.size(); ++i) order_by_path[scene.quads[i].owner_path] = static_cast<int>(i);
    for (auto& rect : scene.rects) {
        auto it = order_by_path.find(rect.path);
        if (it != order_by_path.end()) rect.order = it->second;
    }
}

} // namespace

void layout_container_into(const Instance& container, RectF area, LayoutCtx& ctx,
                           const std::string& path_prefix) {
    UiScene& scene = ctx.scene;
    RectF clip{-1e9f, -1e9f, 2e9f, 2e9f};
    // Layout the container's GUI children (the container itself may be a
    // ScreenGui / SurfaceGui / Frame whose own background fills the area).
    {
        UiRect rec;
        rec.path = path_prefix;
        rec.class_name = container.class_name;
        rec.name = container.name;
        rec.x = area.x; rec.y = area.y; rec.width = area.w; rec.height = area.h;
        rec.visible = true;
        scene.rects.push_back(rec);
        std::vector<int> zkey{static_cast<int>(uenum(container, "DisplayOrder", 0)), 1};
        emit_own_quads(container, area, area, clip, 1.0f, zkey, path_prefix, ctx);
    }
    std::vector<const Instance*> children;
    for (const auto& child : container.children)
        if (is_gui_object(child->class_name)) children.push_back(child.get());
    std::stable_sort(children.begin(), children.end(), [](const Instance* a, const Instance* b) {
        return ufloat(*a, "LayoutOrder", 0) < ufloat(*b, "LayoutOrder", 0);
    });
    std::vector<int> zkey{static_cast<int>(uenum(container, "DisplayOrder", 0)), 1};
    // Honour a layout on the container itself (list/grid), else free layout.
    const Instance* list = decoration(container, "UIListLayout");
    const Instance* grid = decoration(container, "UIGridLayout");
    if (list && !children.empty()) {
        std::vector<RectF> measured;
        for (const Instance* child : children)
            measured.push_back(layout_object(*child, area, 1.0f, clip, 1.0f, true, zkey, false, path_prefix, ctx, nullptr));
        int64_t fill_dir = uenum(*list, "FillDirection", 0);
        auto padding = uudim(*list, "Padding", {0, 0});
        bool horizontal = fill_dir == 1;
        float pad_px = static_cast<float>(horizontal ? padding[0] * area.w + padding[1] : padding[0] * area.h + padding[1]);
        float total = 0;
        for (size_t i = 0; i < measured.size(); ++i) { total += horizontal ? measured[i].w : measured[i].h; if (i) total += pad_px; }
        int64_t halign = uenum(*list, "HorizontalAlignment", 0);
        int64_t valign = uenum(*list, "VerticalAlignment", 0);
        float cursor = horizontal ? (halign == 1 ? area.x + (area.w - total) / 2 : halign == 2 ? area.x + (area.w - total) : area.x)
                                  : (valign == 1 ? area.y + (area.h - total) / 2 : valign == 2 ? area.y + (area.h - total) : area.y);
        for (size_t i = 0; i < children.size(); ++i) {
            RectF forced = measured[i];
            if (horizontal) {
                forced.x = cursor;
                forced.y = valign == 1 ? area.y + (area.h - forced.h) / 2 : valign == 2 ? area.y + (area.h - forced.h) : area.y;
                cursor += forced.w + pad_px;
            } else {
                forced.y = cursor;
                forced.x = halign == 1 ? area.x + (area.w - forced.w) / 2 : halign == 2 ? area.x + (area.w - forced.w) : area.x;
                cursor += forced.h + pad_px;
            }
            layout_object(*children[i], area, 1.0f, clip, 1.0f, true, zkey, true, path_prefix, ctx, &forced);
        }
    } else if (grid && !children.empty()) {
        auto cell = uudim2(*grid, "CellSize", {0, 100, 0, 100});
        auto cell_pad = uudim2(*grid, "CellPadding", {0, 0, 0, 0});
        float cell_w = area.w * static_cast<float>(cell[0]) + static_cast<float>(cell[1]);
        float cell_h = area.h * static_cast<float>(cell[2]) + static_cast<float>(cell[3]);
        float pad_x = area.w * static_cast<float>(cell_pad[0]) + static_cast<float>(cell_pad[1]);
        float pad_y = area.h * static_cast<float>(cell_pad[2]) + static_cast<float>(cell_pad[3]);
        int cols = std::max(1, static_cast<int>((area.w + pad_x) / std::max(cell_w + pad_x, 1.0f)));
        for (size_t i = 0; i < children.size(); ++i) {
            RectF cell_rect{area.x + (static_cast<int>(i) % cols) * (cell_w + pad_x),
                            area.y + (static_cast<int>(i) / cols) * (cell_h + pad_y), cell_w, cell_h};
            layout_object(*children[i], cell_rect, 1.0f, clip, 1.0f, true, zkey, true, path_prefix, ctx, &cell_rect);
        }
    } else {
        for (const Instance* child : children) {
            layout_object(*child, area, 1.0f, clip, 1.0f, true, zkey, true, path_prefix, ctx, nullptr);
        }
    }
} // namespace

UiScene build_ui_layout(const Instance& container, float area_x, float area_y,
                        float area_w, float area_h, const UiOptions& options,
                        const std::string& path_prefix) {
    UiScene scene;
    scene.viewport_w = options.width;
    scene.viewport_h = options.height;
    std::vector<TextJob> jobs;
    LayoutCtx ctx(options, scene, jobs);
    RectF area{area_x, area_y, area_w, area_h};
    layout_container_into(container, area, ctx, path_prefix);
    finish_ui_scene(ctx, scene);
    return scene;
}

UiScene build_ui_scene(const DataModel& model, const UiOptions& options) {
    UiScene scene;
    scene.viewport_w = options.width;
    scene.viewport_h = options.height;
    std::vector<TextJob> jobs;
    LayoutCtx ctx(options, scene, jobs);
    RectF full_clip{-1e9f, -1e9f, 2e9f, 2e9f};

    // ---- ScreenGuis ----
    std::vector<const Instance*> screen_guis;
    if (!options.screen_gui.empty()) {
        std::string error;
        if (Instance* found = model.resolve_path(options.screen_gui, error)) {
            if (found->class_name == "ScreenGui") screen_guis.push_back(found);
            else scene.warnings.push_back("'" + options.screen_gui + "' is a " + found->class_name + ", not a ScreenGui");
        } else {
            scene.warnings.push_back("ScreenGui '" + options.screen_gui + "' not found: " + error);
        }
    } else if (const Instance* starter = find_service(model, "StarterGui")) {
        for (const auto& child : starter->children)
            if (child->class_name == "ScreenGui") screen_guis.push_back(child.get());
    }
    std::stable_sort(screen_guis.begin(), screen_guis.end(), [](const Instance* a, const Instance* b) {
        double da = ufloat(*a, "DisplayOrder", 0), db = ufloat(*b, "DisplayOrder", 0);
        return da != db ? da < db : a->name < b->name;
    });
    for (const Instance* gui : screen_guis) {
        std::string prefix = "StarterGui/" + gui->name;
        bool enabled = ubool(*gui, "Enabled", true);
        float inset = 0;
        if (!options.ignore_gui_inset && !ubool(*gui, "IgnoreGuiInset", false) &&
            uenum(*gui, "ScreenInsets", 2) != 0) {
            inset = options.topbar_inset;
        }
        RectF area{0, inset, static_cast<float>(options.width),
                   static_cast<float>(options.height) - inset};
        if (!enabled) {
            UiRect rec;
            rec.path = prefix;
            rec.class_name = "ScreenGui";
            rec.name = gui->name;
            rec.x = area.x; rec.y = area.y; rec.width = area.w; rec.height = area.h;
            rec.visible = false;
            scene.rects.push_back(rec);
            continue;
        }
        layout_container_into(*gui, area, ctx, prefix);
    }

    // ---- BillboardGuis (projected with the configured camera) ----
    {
        // Simple look-at basis for projection.
        auto sub3 = [](std::array<float, 3> a, std::array<float, 3> b) {
            return std::array<float, 3>{a[0] - b[0], a[1] - b[1], a[2] - b[2]};
        };
        auto cross3 = [](std::array<float, 3> a, std::array<float, 3> b) {
            return std::array<float, 3>{a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2],
                                        a[0] * b[1] - a[1] * b[0]};
        };
        auto norm3 = [](std::array<float, 3> a) {
            float l = std::sqrt(a[0] * a[0] + a[1] * a[1] + a[2] * a[2]);
            return l > 0 ? std::array<float, 3>{a[0] / l, a[1] / l, a[2] / l} : a;
        };
        auto fwd = norm3(sub3(options.camera_look_at, options.camera_position));
        auto right = norm3(cross3(fwd, {0, 1, 0}));
        auto up = cross3(right, fwd);
        float tan_half = std::tan(options.camera_fov_degrees * 3.14159265f / 360.0f);
        float aspect = options.height > 0 ? static_cast<float>(options.width) / options.height : 1.0f;

        std::vector<const Instance*> billboards;
        std::function<void(const Instance&)> collect = [&](const Instance& inst) {
            if (inst.class_name == "BillboardGui") billboards.push_back(&inst);
            for (const auto& child : inst.children) collect(*child);
        };
        collect(model.root());
        for (const Instance* bb : billboards) {
            std::string path = model.path_of(*bb);
            if (!ubool(*bb, "Enabled", true)) continue;
            const Instance* adornee = nullptr;
            if (const Variant* ref = uprop(*bb, "Adornee")) {
                if (ref->type == ValueType::Ref) adornee = resolve_index_path(model.root(), ref->string_value);
            }
            if (!adornee && bb->parent && DataModel::is_base_part(bb->parent->class_name)) adornee = bb->parent;
            if (!adornee) {
                scene.warnings.push_back(path + ": BillboardGui has no resolvable Adornee; not drawn");
                continue;
            }
            const Variant* cfv = uprop(*adornee, "CFrame");
            if (!cfv || cfv->type != ValueType::CFrame) continue;
            auto studs = uvec2(*bb, "StudsOffset", {0, 0});
            auto studs3 = uvec2(*bb, "StudsOffsetWorldSpace", {0, 0});
            (void)studs3;
            std::array<float, 3> world{static_cast<float>(cfv->position[0]) + static_cast<float>(studs[0]),
                                       static_cast<float>(cfv->position[1]) + static_cast<float>(studs[1]),
                                       static_cast<float>(cfv->position[2])};
            auto rel = sub3(world, options.camera_position);
            float cz = rel[0] * fwd[0] + rel[1] * fwd[1] + rel[2] * fwd[2];
            UiRect rec;
            rec.path = path;
            rec.class_name = "BillboardGui";
            rec.name = bb->name;
            if (cz <= 0.1f) {
                rec.visible = false;
                scene.rects.push_back(rec);
                continue;
            }
            float cx = rel[0] * right[0] + rel[1] * right[1] + rel[2] * right[2];
            float cy = rel[0] * up[0] + rel[1] * up[1] + rel[2] * up[2];
            float ndc_x = cx / (cz * tan_half * aspect);
            float ndc_y = cy / (cz * tan_half);
            float sx = (ndc_x * 0.5f + 0.5f) * options.width;
            float sy = (0.5f - ndc_y * 0.5f) * options.height;
            auto size_spec = uudim2(*bb, "Size", {0, 200, 0, 50});
            float w = size_spec[0] * options.width + size_spec[1];
            float h = size_spec[2] * options.height + size_spec[3];
            RectF area{sx - w / 2, sy - h, w, h};
            rec.x = area.x; rec.y = area.y; rec.width = w; rec.height = h;
            rec.visible = true;
            scene.rects.push_back(rec);
            layout_container_into(*bb, area, ctx, path);
        }
    }

    finish_ui_scene(ctx, scene);
    return scene;
}

std::vector<UiIssue> validate_ui(const UiScene& scene, const DataModel& model) {
    std::vector<UiIssue> issues;
    for (const auto& w : scene.warnings) {
        // Layout warnings reference their object path before the colon.
        std::string path = w.substr(0, w.find(':'));
        issues.push_back({"warning", path, w});
    }
    float vw = static_cast<float>(scene.viewport_w), vh = static_cast<float>(scene.viewport_h);
    std::map<std::string, std::vector<const UiRect*>> by_parent;
    for (const auto& rect : scene.rects) {
        if (!rect.visible) continue;
        by_parent[rect.parent_path].push_back(&rect);
        if (rect.width <= 0 || rect.height <= 0) {
            issues.push_back({"warning", rect.path, "zero or negative size (" +
                              std::to_string(static_cast<int>(rect.width)) + "x" +
                              std::to_string(static_cast<int>(rect.height)) + ")"});
        }
        bool fully_out = rect.x + rect.width < 0 || rect.y + rect.height < 0 || rect.x > vw || rect.y > vh;
        bool partially_out = rect.x < 0 || rect.y < 0 || rect.x + rect.width > vw || rect.y + rect.height > vh;
        if (fully_out) issues.push_back({"error", rect.path, "element is fully offscreen at this viewport"});
        else if (partially_out) issues.push_back({"warning", rect.path, "element is partially offscreen at this viewport"});
    }
    for (const auto& [parent, siblings] : by_parent) {
        for (size_t i = 0; i < siblings.size(); ++i) {
            for (size_t j = i + 1; j < siblings.size(); ++j) {
                const UiRect& a = *siblings[i];
                const UiRect& b = *siblings[j];
                float ox = std::min(a.x + a.width, b.x + b.width) - std::max(a.x, b.x);
                float oy = std::min(a.y + a.height, b.y + b.height) - std::max(a.y, b.y);
                if (ox <= 0 || oy <= 0) continue;
                float smaller = std::min(a.width * a.height, b.width * b.height);
                if (smaller > 0 && (ox * oy) / smaller > 0.5f) {
                    issues.push_back({"warning", a.path, "overlaps sibling '" + b.name + "' by more than 50% of the smaller element"});
                }
            }
        }
    }
    // Model-level unsupported features.
    std::function<void(const Instance&)> walk = [&](const Instance& inst) {
        if (is_gui_object(inst.class_name) || inst.class_name == "BillboardGui" || inst.class_name == "SurfaceGui") {
            std::string path = model.path_of(inst);
            if (std::abs(ufloat(inst, "Rotation", 0.0)) > 0.001)
                issues.push_back({"warning", path, "Rotation is not rendered in Phase 4"});
            if (inst.class_name == "SurfaceGui")
                issues.push_back({"warning", path, "SurfaceGui is rendered as a texture in the 3D pass; check it with `render`, not `ui render`"});
        }
        for (const auto& child : inst.children) walk(*child);
    };
    walk(model.root());
    return issues;
}

// ===================== Software rasterizer (SurfaceGui) =====================
namespace {

bool load_png_rgba_file(const std::string& path, uint32_t& w, uint32_t& h, std::vector<uint8_t>& out);

void blend_pixel(std::vector<uint8_t>& img, uint32_t iw, uint32_t ih, int x, int y,
                 const std::array<float, 4>& src) {
    if (x < 0 || y < 0 || x >= static_cast<int>(iw) || y >= static_cast<int>(ih)) return;
    size_t at = (static_cast<size_t>(y) * iw + x) * 4;
    float sa = std::clamp(src[3], 0.0f, 1.0f);
    for (int c = 0; c < 3; ++c) {
        float dst = img[at + c] / 255.0f;
        float blended = src[c] * sa + dst * (1 - sa);
        img[at + c] = static_cast<uint8_t>(std::clamp(blended, 0.0f, 1.0f) * 255.0f + 0.5f);
    }
    float da = img[at + 3] / 255.0f;
    img[at + 3] = static_cast<uint8_t>(std::clamp(sa + da * (1 - sa), 0.0f, 1.0f) * 255.0f + 0.5f);
}

bool load_png_rgba_file(const std::string& path, uint32_t& w, uint32_t& h, std::vector<uint8_t>& out) {
    // Minimal PNG reader via libpng (same conversion rules as the renderer).
    FILE* fp = fopen(path.c_str(), "rb");
    if (!fp) return false;
    unsigned char header[8];
    if (fread(header, 1, 8, fp) != 8 || png_sig_cmp(header, 0, 8)) { fclose(fp); return false; }
    png_structp png_ptr = png_create_read_struct(PNG_LIBPNG_VER_STRING, nullptr, nullptr, nullptr);
    png_infop info_ptr = png_create_info_struct(png_ptr);
    if (setjmp(png_jmpbuf(png_ptr))) {
        png_destroy_read_struct(&png_ptr, &info_ptr, nullptr);
        fclose(fp);
        return false;
    }
    png_init_io(png_ptr, fp);
    png_set_sig_bytes(png_ptr, 8);
    png_read_info(png_ptr, info_ptr);
    w = png_get_image_width(png_ptr, info_ptr);
    h = png_get_image_height(png_ptr, info_ptr);
    png_set_expand(png_ptr);
    png_set_strip_16(png_ptr);
    png_set_add_alpha(png_ptr, 0xFF, PNG_FILLER_AFTER);
    png_read_update_info(png_ptr, info_ptr);
    out.resize(static_cast<size_t>(w) * h * 4);
    std::vector<png_bytep> rows(h);
    for (uint32_t y = 0; y < h; ++y) rows[y] = out.data() + static_cast<size_t>(y) * w * 4;
    png_read_image(png_ptr, rows.data());
    png_destroy_read_struct(&png_ptr, &info_ptr, nullptr);
    fclose(fp);
    return true;
}

} // namespace

std::vector<uint8_t> rasterize_ui(const UiScene& scene, uint32_t width, uint32_t height,
                                  const std::array<float, 4>& background) {
    std::vector<uint8_t> img(static_cast<size_t>(width) * height * 4);
    for (size_t i = 0; i < img.size(); i += 4) {
        img[i + 0] = static_cast<uint8_t>(background[0] * 255.0f + 0.5f);
        img[i + 1] = static_cast<uint8_t>(background[1] * 255.0f + 0.5f);
        img[i + 2] = static_cast<uint8_t>(background[2] * 255.0f + 0.5f);
        img[i + 3] = static_cast<uint8_t>(background[3] * 255.0f + 0.5f);
    }
    std::map<std::string, std::vector<uint8_t>> image_cache;
    std::map<std::string, std::array<uint32_t, 2>> image_sizes;
    for (const auto& quad : scene.quads) {
        int x0 = std::max(0, static_cast<int>(std::floor(quad.x)));
        int y0 = std::max(0, static_cast<int>(std::floor(quad.y)));
        int x1 = std::min(static_cast<int>(width), static_cast<int>(std::ceil(quad.x + quad.width)));
        int y1 = std::min(static_cast<int>(height), static_cast<int>(std::ceil(quad.y + quad.height)));
        // Texture fetch.
        const std::vector<uint8_t>* tex = nullptr;
        uint32_t tw = 0, th = 0;
        if (quad.use_atlas && scene.atlas_w > 0) {
            tex = &scene.atlas_rgba;
            tw = scene.atlas_w;
            th = scene.atlas_h;
        } else if (!quad.texture_path.empty()) {
            auto it = image_cache.find(quad.texture_path);
            if (it == image_cache.end()) {
                std::vector<uint8_t> pixels;
                uint32_t w = 0, h = 0;
                if (load_png_rgba_file(quad.texture_path, w, h, pixels)) {
                    image_sizes[quad.texture_path] = {w, h};
                    image_cache[quad.texture_path] = std::move(pixels);
                    tex = &image_cache[quad.texture_path];
                    tw = w; th = h;
                }
            } else {
                tex = &it->second;
                tw = image_sizes[quad.texture_path][0];
                th = image_sizes[quad.texture_path][1];
            }
        }
        for (int y = y0; y < y1; ++y) {
            for (int x = x0; x < x1; ++x) {
                float lx = x - quad.x, ly = y - quad.y;
                // Clip.
                if (x < quad.clip_x || y < quad.clip_y || x >= quad.clip_x + quad.clip_w ||
                    y >= quad.clip_y + quad.clip_h) continue;
                // Rounded corners: distance to the rounded-rect SDF.
                if (quad.corner_radius > 0) {
                    float r = std::min(quad.corner_radius, std::min(quad.width, quad.height) / 2);
                    float cx = std::clamp(lx, r, quad.width - r);
                    float cy = std::clamp(ly, r, quad.height - r);
                    float dx = lx - cx, dy = ly - cy;
                    if (dx * dx + dy * dy > r * r) continue;
                }
                std::array<float, 4> colour = quad.color;
                if (quad.has_gradient) {
                    float rad = quad.gradient_degrees * 3.14159265f / 180.0f;
                    float gx = std::cos(rad), gy = std::sin(rad);
                    float t = (lx * gx + ly * gy) / std::max(quad.width + quad.height, 1.0f) + 0.5f;
                    t = std::clamp(t, 0.0f, 1.0f);
                    for (int c = 0; c < 4; ++c) colour[c] = quad.color[c] + (quad.gradient_end[c] - quad.color[c]) * t;
                }
                if (tex && tw > 0 && th > 0) {
                    float u = quad.u0 + (lx / std::max(quad.width, 1.0f)) * (quad.u1 - quad.u0);
                    float v = quad.v0 + (ly / std::max(quad.height, 1.0f)) * (quad.v1 - quad.v0);
                    u = u - std::floor(u);
                    v = v - std::floor(v);
                    uint32_t tx = std::min(static_cast<uint32_t>(u * tw), tw - 1);
                    uint32_t ty = std::min(static_cast<uint32_t>(v * th), th - 1);
                    size_t at = (static_cast<size_t>(ty) * tw + tx) * 4;
                    std::array<float, 4> sampled{(*tex)[at] / 255.0f, (*tex)[at + 1] / 255.0f,
                                                 (*tex)[at + 2] / 255.0f, (*tex)[at + 3] / 255.0f};
                    colour = {colour[0] * sampled[0], colour[1] * sampled[1],
                              colour[2] * sampled[2], colour[3] * sampled[3]};
                }
                blend_pixel(img, width, height, x, y, colour);
            }
        }
    }
    return img;
}

} // namespace blox
