#pragma once
// Internal seam: PNG decoding shared by renderer.cpp (textures) and
// image.cpp (write_png/image_diff). Not a public API.
#include <cstdint>
#include <string>
#include <vector>

namespace blox {
bool load_png_rgba(const std::string& path, uint32_t& w, uint32_t& h,
                   std::vector<uint8_t>& out, std::string& error);
} // namespace blox
