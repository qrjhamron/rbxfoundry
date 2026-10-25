// PNG image I/O and pixel-difference metrics. Separated from the Vulkan
// renderer so non-render callers (parity/image-diff, UI rasterizer tests)
// do not depend on Vulkan device code. load_png_rgba is shared with the
// renderer's texture upload path via image_internal.hpp.
#include "blox/rendering/renderer.hpp"
#include "image_internal.hpp"

#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <png.h>

namespace blox {

bool load_png_rgba(const std::string& path, uint32_t& w, uint32_t& h, std::vector<uint8_t>& out,
                   std::string& error) {
    FILE* fp = fopen(path.c_str(), "rb");
    if (!fp) { error = "cannot open texture '" + path + "'"; return false; }
    unsigned char header[8];
    if (fread(header, 1, 8, fp) != 8 || png_sig_cmp(header, 0, 8)) {
        fclose(fp);
        error = "texture '" + path + "' is not a PNG (only PNG textures are supported)";
        return false;
    }
    png_structp png_ptr = png_create_read_struct(PNG_LIBPNG_VER_STRING, nullptr, nullptr, nullptr);
    png_infop info_ptr = png_create_info_struct(png_ptr);
    if (setjmp(png_jmpbuf(png_ptr))) {
        png_destroy_read_struct(&png_ptr, &info_ptr, nullptr);
        fclose(fp);
        error = "failed to decode PNG texture '" + path + "'";
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

bool write_png(const std::string& path, uint32_t width, uint32_t height,
               const std::vector<uint8_t>& rgba, std::string& error) {
    if (rgba.size() < static_cast<size_t>(width) * height * 4) {
        error = "pixel buffer too small for PNG";
        return false;
    }
    FILE* fp = fopen(path.c_str(), "wb");
    if (!fp) { error = "cannot open '" + path + "' for writing"; return false; }
    png_structp png_ptr = png_create_write_struct(PNG_LIBPNG_VER_STRING, nullptr, nullptr, nullptr);
    if (!png_ptr) { fclose(fp); error = "png_create_write_struct failed"; return false; }
    png_infop info_ptr = png_create_info_struct(png_ptr);
    if (setjmp(png_jmpbuf(png_ptr))) {
        png_destroy_write_struct(&png_ptr, &info_ptr);
        fclose(fp);
        error = "PNG encode failed for '" + path + "'";
        return false;
    }
    png_init_io(png_ptr, fp);
    png_set_IHDR(png_ptr, info_ptr, width, height, 8, PNG_COLOR_TYPE_RGBA,
                 PNG_INTERLACE_NONE, PNG_COMPRESSION_TYPE_DEFAULT, PNG_FILTER_TYPE_DEFAULT);
    png_write_info(png_ptr, info_ptr);
    for (uint32_t y = 0; y < height; ++y) {
        png_write_row(png_ptr, rgba.data() + static_cast<size_t>(y) * width * 4);
    }
    png_write_end(png_ptr, nullptr);
    png_destroy_write_struct(&png_ptr, &info_ptr);
    fclose(fp);
    return true;
}

bool image_diff(const std::string& path_a, const std::string& path_b,
                const std::string& heatmap_path, ImageDiff& out, std::string& error) {
    uint32_t wa = 0, ha = 0, wb = 0, hb = 0;
    std::vector<uint8_t> a, b;
    if (!load_png_rgba(path_a, wa, ha, a, error)) return false;
    if (!load_png_rgba(path_b, wb, hb, b, error)) return false;
    if (wa != wb || ha != hb) {
        error = "image sizes differ: " + std::to_string(wa) + "x" + std::to_string(ha) + " vs " +
                std::to_string(wb) + "x" + std::to_string(hb);
        return false;
    }
    out.width = wa;
    out.height = ha;
    out.diff_pixels = 0;
    out.max_delta = 0;
    double sum_abs = 0;
    double sum_sq = 0;
    const size_t pixels = static_cast<size_t>(wa) * ha;
    std::vector<uint8_t> heat;
    if (!heatmap_path.empty()) heat.assign(pixels * 4, 0);
    for (size_t i = 0; i < pixels; ++i) {
        int pixel_max = 0;
        for (int c = 0; c < 4; ++c) {
            int d = std::abs(static_cast<int>(a[i * 4 + c]) - static_cast<int>(b[i * 4 + c]));
            sum_abs += d;
            sum_sq += static_cast<double>(d) * d;
            pixel_max = std::max(pixel_max, d);
        }
        if (pixel_max > 0) ++out.diff_pixels;
        out.max_delta = std::max(out.max_delta, pixel_max);
        if (!heat.empty()) {
            heat[i * 4 + 0] = static_cast<uint8_t>(pixel_max);
            heat[i * 4 + 1] = static_cast<uint8_t>(pixel_max / 4);
            heat[i * 4 + 2] = 0;
            heat[i * 4 + 3] = 255;
        }
    }
    const double channels = static_cast<double>(pixels) * 4.0;
    out.mae = channels > 0 ? sum_abs / channels : 0;
    out.rmse = channels > 0 ? std::sqrt(sum_sq / channels) : 0;
    if (!heatmap_path.empty() && !write_png(heatmap_path, wa, ha, heat, error)) return false;
    return true;
}


} // namespace blox
