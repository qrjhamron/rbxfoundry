#pragma once

#include <string>
#include <vector>

namespace blox {

// One diagnostic from the official Luau frontend (Phase 23):
// syntax errors, type errors and lint warnings with source spans.
struct ScriptDiagnostic {
    std::string severity; // "error" | "warning"
    std::string kind;     // "type" | "syntax" | "lint"
    std::string message;
    int line = 0;       // 1-based
    int column = 0;     // 1-based
    int end_line = 0;
    int end_column = 0;
};

// Runs the official Luau type checker (Luau::Frontend, new solver)
// over a single in-memory module. Roblox globals (game, Instance, ...)
// are NOT declared, so references to them surface as unknown-global
// type errors — callers must present results accordingly (the runtime
// accepts them; the checker is a strict Luau oracle, not a Roblox one).
std::vector<ScriptDiagnostic> analyze_luau(const std::string& source,
                                           const std::string& module_name);

} // namespace blox
