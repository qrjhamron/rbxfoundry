#pragma once
// Internal seam between the GUI text engine (text.cpp) and the layout
// engine (layout.cpp). Not a public API.
#include <cstdint>

namespace blox {
bool is_cjk_codepoint_for_layout(char32_t cp);
} // namespace blox
