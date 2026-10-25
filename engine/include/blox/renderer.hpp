#pragma once
#include <array>
#include <cmath>
#include <cstdint>
#include <string>
#include <vector>

namespace blox {

struct UiScene;
struct UiPipeline;

// One drawable extracted from the DataModel.
struct DrawItem {
    std::vector<std::array<float, 3>> positions; // world-space after model transform applied by renderer? No:
    // Geometry is stored LOCAL (already sized), transform in `model`.
    std::vector<std::array<float, 3>> normals;
    std::vector<std::array<float, 2>> uvs;
    std::vector<uint32_t> indices;
    std::array<float, 16> model{}; // column-major CFrame matrix
    std::array<float, 3> color{1, 1, 1}; // linear-space RGB
    float transparency = 0.0f;
    uint32_t material = 256; // Enum.Material value
    std::string texture_path;  // empty = untextured (Phase 3C)
    // SurfaceAppearance PBR maps (Phase 11); empty = absent.
    std::string normal_path;
    std::string roughness_path;
    std::string metalness_path;
    // Camera-facing quad: local XY corners expanded by the renderer
    // around the model translation (particles).
    bool billboard = false;
    std::string warning;       // non-fatal fidelity note (e.g. unskinned mesh)
    std::string name;
    bool cast_shadow = true;
    bool receive_shadow = true;
};

struct CameraInfo {
    std::array<float, 3> position{30, 25, 30};
    std::array<float, 3> look_at{0, 0, 0};
    std::array<float, 3> up{0, 1, 0};
    float fov_degrees = 70.0f; // vertical
    float near_plane = 0.1f;
    float far_plane = 1000.0f;
    bool orthographic = false;
    float ortho_height = 40.0f; // world units visible vertically (ortho)
};

// A dynamic (non-sun) light extracted from PointLight / SpotLight /
// SurfaceLight instances. cone_cos < -1 marks an omni point light.
struct DynamicLight {
    std::array<float, 3> position{0, 0, 0};
    std::array<float, 3> color{1, 1, 1}; // linear, pre-scaled by brightness
    float range = 8.0f;
    std::array<float, 3> direction{0, -1, 0};
    float cone_cos = -2.0f;
};

struct LightingInfo {
    std::array<float, 3> ambient{0.2f, 0.2f, 0.2f};        // linear
    std::array<float, 3> outdoor_ambient{0.5f, 0.5f, 0.5f}; // linear (hemisphere sky)
    std::array<float, 3> sun_direction{0.4f, -0.8f, 0.45f}; // normalized, pointing down-ish
    std::array<float, 3> sun_color{1.0f, 0.95f, 0.88f};     // linear, includes Brightness scale
    std::array<float, 3> background{0.55f, 0.65f, 0.75f};   // sRGB-ish clear colour (converted)
    bool shadows_enabled = true;
    uint32_t shadow_map_size = 2048;
    float fog_start = 1e9f;
    float fog_end = 1e9f;
    std::array<float, 3> fog_color{0.55f, 0.65f, 0.75f};
    float exposure = 1.0f;
    std::vector<DynamicLight> dynamic_lights; // capped at 8 in the shader
    bool has_sky = false;
    std::array<float, 3> sky_top{0.35f, 0.55f, 0.85f};
    std::array<float, 3> sky_bottom{0.75f, 0.82f, 0.9f};
};

struct Scene {
    std::vector<DrawItem> draws;
    LightingInfo lighting;
    CameraInfo current_camera;   // from Workspace.CurrentCamera if present
    bool has_current_camera = false;
    std::string current_camera_name;
    std::vector<std::string> warnings;       // rendered-with-approximation notes
    std::vector<std::string> missing_assets; // MeshIds/textures that could not be resolved
    std::vector<std::string> skipped;        // instances skipped with reasons (e.g. MeshPart w/o mesh in 3A)
    uint32_t part_count = 0;
};

struct RenderRequest {
    uint32_t width = 800;
    uint32_t height = 600;
    CameraInfo camera;
    std::string output_path; // PNG destination (written by the engine)
    bool shadows = true;
    uint32_t shadow_map_size = 2048;
    uint32_t msaa = 1; // 1 = off
    // When true, also returns the raw RGBA pixels (for agent probing/tests)
    bool return_pixels = false;
};

struct RenderResult {
    bool ok = false;
    std::string error;
    std::string device_name;
    uint32_t width = 0;
    uint32_t height = 0;
    uint32_t draw_calls = 0;
    uint32_t triangles = 0;
    double render_ms = 0.0;
    std::vector<uint8_t> rgba; // only when return_pixels requested
};

struct UiRenderRequest {
    uint32_t width = 1280;
    uint32_t height = 720;
    std::string output_path;
    std::array<float, 4> background{0, 0, 0, 0}; // straight RGBA clear colour
    // Optional already-rendered 3D frame (RGBA8, width*height*4) drawn
    // behind the UI (composite screenshots).
    std::vector<uint8_t> background_rgba;
    bool return_pixels = false;
};

// Headless Vulkan renderer. One instance renders one scene at a time;
// instance/device are created once and reused across calls.
class Renderer {
public:
    Renderer();
    ~Renderer();
    Renderer(const Renderer&) = delete;
    Renderer& operator=(const Renderer&) = delete;

    // Lazily initialises Vulkan. Returns false + error when unavailable
    // (no driver), which callers must surface as a hard failure.
    bool init(std::string& error);
    bool available() const { return initialized_; }
    const std::string& device_name() const { return device_name_; }

    RenderResult render(const Scene& scene, const RenderRequest& request);
    // Draws a laid-out UiScene (blox/ui.hpp) with the Vulkan UI pipeline.
    RenderResult render_ui(const UiScene& scene, const UiRenderRequest& request);

private:
    bool ensure_pipelines(std::string& error);
    bool ensure_ui_pipeline(UiPipeline& ui, std::string& error);
    bool initialized_ = false;
    std::string device_name_;
    struct Impl;
    Impl* impl_ = nullptr;
};

// Encodes RGBA8 pixels as a PNG file (deterministic: no timestamps).
bool write_png(const std::string& path, uint32_t width, uint32_t height,
               const std::vector<uint8_t>& rgba, std::string& error);

// sRGB (0..1, as stored in Color3 properties) to linear space.
inline std::array<float, 3> srgb_to_linear(const std::array<float, 3>& c) {
    auto f = [](float v) {
        return v <= 0.04045f ? v / 12.92f : std::pow((v + 0.055f) / 1.055f, 2.4f);
    };
    return {f(c[0]), f(c[1]), f(c[2])};
}

} // namespace blox
