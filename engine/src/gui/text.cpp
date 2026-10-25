#include "blox/gui/ui.hpp"
#include "text_internal.hpp"

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

// Process-lifetime chain cache: layout measures every word of every
// label (and re-measures during wrap/TextScaled searches), so loading
// four faces per call dominated text-heavy renders (measured: ~1.5 s
// per 60-label frame before this cache). Faces are shared read-only
// apart from pixel size, which is set per call.
FaceChain& cached_chain(const std::string& font_path) {
    static thread_local std::map<std::string, FaceChain> cache;
    auto it = cache.find(font_path);
    if (it == cache.end()) {
        it = cache.emplace(font_path, load_chain(font_path)).first;
    }
    return it->second;
}

float measure_text(const std::string& text, float size_px, const std::string& font_path) {
    FaceChain& chain = cached_chain(font_path);
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

    // Per-render glyph cache: repeated codepoints (the common case in
    // real UI text) are rasterized and packed once, then re-emitted.
    struct CachedGlyph {
        std::array<float, 4> rect{}; // pixel-space atlas rect
        float bearing_x = 0, bearing_y = 0, advance = 0;
        uint32_t w = 0, h = 0;
        bool empty = false;
    };
    std::map<std::tuple<std::string, char32_t, int>, CachedGlyph> glyph_cache;

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
            auto cache_key = std::make_tuple(job.font_path, cp, static_cast<int>(job.size_px * 4));
            if (auto cached = glyph_cache.find(cache_key); cached != glyph_cache.end()) {
                const CachedGlyph& g = cached->second;
                float gx = pen_x + g.bearing_x;
                float gy = baseline - g.bearing_y;
                pen_x += g.advance;
                if (g.empty) continue;
                if (gx + g.w < job.clip_x || gx > job.clip_x + job.clip_w ||
                    gy + g.h < job.clip_y || gy > job.clip_y + job.clip_h) {
                    continue;
                }
                UiQuad quad;
                quad.x = gx;
                quad.y = gy;
                quad.width = static_cast<float>(g.w);
                quad.height = static_cast<float>(g.h);
                quad.u0 = g.rect[0] / atlas_w;
                quad.u1 = g.rect[2] / atlas_w;
                quad.v0 = g.rect[1];
                quad.v1 = g.rect[3];
                quad.color = job.color;
                quad.use_atlas = true;
                quad.sort_key = job.sort_key;
                quad.owner_path = job.owner_path;
                quad.sequence = job.sequence;
                quads.push_back(quad);
                continue;
            }
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
            if (bmp.width == 0 || bmp.rows == 0) {
                CachedGlyph g;
                g.empty = true;
                g.bearing_x = static_cast<float>(glyph->bitmap_left);
                g.bearing_y = static_cast<float>(glyph->bitmap_top);
                g.advance = static_cast<float>(glyph->advance.x >> 6);
                glyph_cache[cache_key] = g;
                continue;
            }
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
            {
                CachedGlyph g;
                g.rect = rect;
                g.bearing_x = static_cast<float>(glyph->bitmap_left);
                g.bearing_y = static_cast<float>(glyph->bitmap_top);
                g.advance = static_cast<float>(glyph->advance.x >> 6);
                g.w = bmp.width;
                g.h = bmp.rows;
                glyph_cache[cache_key] = g;
            }
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

bool is_cjk_codepoint_for_layout(char32_t cp) { return is_cjk_codepoint(cp); }

} // namespace blox
