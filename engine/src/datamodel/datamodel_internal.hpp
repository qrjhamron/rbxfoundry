#pragma once
// Internal helpers shared by the datamodel translation units
// (datamodel.cpp, reflection.cpp, transforms.cpp). Not a public API.
#include <string>
#include <vector>
#include "blox/datamodel/datamodel.hpp"

namespace blox::detail {
void sync_position_from_cframe(Instance& inst);
std::vector<std::string> split_path(const std::string& path);
} // namespace blox::detail
