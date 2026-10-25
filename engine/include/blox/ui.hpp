#pragma once
#include <array>
#include <set>
#include <string>
#include <vector>

#include "blox/datamodel.hpp"
#include "blox/renderer.hpp"

namespace blox {

// Computed absolute layout for one GUI object (pixels, origin top-left).
struct UiRect {
    std::string path;
    std::string class_name;
    std::string name;
    float x = 0, y = 0, width = 0, height = 0;
    float zindex = 1;
    bool visible = true;
    bool clips_descendants = false;
    std::string parent_path;
    int order = 0; // render order index after sorting (0 = drawn first)
    std::string text; // for text-bearing objects (debug/inspection)
};

struct UiQuad {
    // Pixel-space rectangle with UVs into the shared texture (glyph atlas or image).
    float x = 0, y = 0, width = 0, height = 0;
    float u0 = 0, v0 = 0, u1 = 1, v1 = 1;
    std::array<float, 4> color{1, 1, 1, 1}; // RGBA straight, sRGB values 0..1
    std::string texture_path;               // empty = glyph atlas / solid
    bool use_atlas = false;                 // glyphs
    float corner_radius = 0;                // rounded-rect SDF radius (pixels)
    // Optional UIGradient: second colour + rotation (degrees).
    bool has_gradient = false;
    std::array<float, 4> gradient_end{1, 1, 1, 1};
    float gradient_degrees = 0;
    // Clip rectangle (pixels); default = unclipped.
    float clip_x = -1e9f, clip_y = -1e9f, clip_w = 2e9f, clip_h = 2e9f;
    // ZIndex sort key (chain for Sibling behaviour) + insertion sequence.
    std::vector<int> sort_key;
    uint32_t sequence = 0;
    std::string owner_path;
};

struct UiScene {
    std::vector<UiRect> rects;   // every computed GUI object (for inspect/layout)
    std::vector<UiQuad> quads;   // draw order (already ZIndex sorted)
    std::vector<std::string> warnings;
    uint32_t viewport_w = 0;
    uint32_t viewport_h = 0;
    // Glyph atlas produced alongside the quads (white RGB + alpha coverage).
    std::vector<uint8_t> atlas_rgba;
    uint32_t atlas_w = 0;
    uint32_t atlas_h = 0;
};

struct UiOptions {
    uint32_t width = 1280;
    uint32_t height = 720;
    std::string screen_gui;    // optional: only this ScreenGui (name or path); empty = all
    bool ignore_gui_inset = false;
    float topbar_inset = 36.0f; // reserved CoreGui/topbar strip when insets apply
    std::string assets_dir;
    std::string place_dir;
    // Text rendering: path to a TTF; empty = system DejaVu Sans discovery.
    std::string font_path;
    // Camera used to project BillboardGuis (defaults match render framing).
    std::array<float, 3> camera_position{30, 25, 30};
    std::array<float, 3> camera_look_at{0, 0, 0};
    float camera_fov_degrees = 70.0f;
};

// Lays out the GUI children of an arbitrary container instance (a ScreenGui,
// SurfaceGui, or GuiObject) into the given pixel area. Used for ScreenGuis
// by build_ui_scene and for SurfaceGui faces by the 3D scene builder.
UiScene build_ui_layout(const Instance& container, float area_x, float area_y,
                        float area_w, float area_h, const UiOptions& options,
                        const std::string& path_prefix);

// Software-rasterizes a laid-out UI scene to RGBA pixels (used for
// SurfaceGui face textures). Background is a straight RGBA colour.
std::vector<uint8_t> rasterize_ui(const UiScene& scene, uint32_t width, uint32_t height,
                                  const std::array<float, 4>& background);

// Computes layout for every ScreenGui in the DataModel. GUI objects are
// laid out with the Roblox UDim2/AnchorPoint model (Phase 4).
UiScene build_ui_scene(const DataModel& model, const UiOptions& options);

// A computed layout issue for `ui validate`.
struct UiIssue {
    std::string severity; // "error" | "warning"
    std::string path;
    std::string message;
};
std::vector<UiIssue> validate_ui(const UiScene& scene, const DataModel& model);

// Rasterizes text runs for a UiQuad batch. Implemented with FreeType when
// available; glyph bitmaps go into `atlas_rgba` (white RGB + alpha coverage).
struct TextJob {
    std::string text;
    float x, y;          // baseline origin (pixels)
    float size_px;
    std::array<float, 4> color;
    std::string font_path;
    // Clip rectangle (pixels): glyphs outside are dropped at quad level.
    float clip_x = -1e9f, clip_y = -1e9f, clip_w = 2e9f, clip_h = 2e9f;
    std::vector<int> sort_key;
    std::string owner_path;
    uint32_t sequence = 0;
};
bool rasterize_text(const std::vector<TextJob>& jobs, std::vector<UiQuad>& quads,
                    std::vector<uint8_t>& atlas_rgba, uint32_t& atlas_w, uint32_t& atlas_h,
                    std::string& error, std::set<char32_t>* missing_out = nullptr);
// Measures text width in pixels for wrapping/layout (same font path rules).
float measure_text(const std::string& text, float size_px, const std::string& font_path);

} // namespace blox
