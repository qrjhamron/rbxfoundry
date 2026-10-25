// Luau language tooling bridge (Phase 23): runs the official Luau
// Frontend (parser + new type solver + linter) over in-memory source.

#include "blox/scripting/script_tools.hpp"

#include "Luau/ConfigResolver.h"
#include "Luau/Error.h"
#include "Luau/FileResolver.h"
#include "Luau/Frontend.h"
#include "Luau/Linter.h"
#include "Luau/BuiltinDefinitions.h"

namespace blox {
namespace {

struct StringFileResolver : Luau::FileResolver {
    std::string module_name;
    std::string source;

    std::optional<Luau::SourceCode> readSource(const Luau::ModuleName& name) override {
        if (name == module_name) {
            return Luau::SourceCode{source, Luau::SourceCode::Module};
        }
        return std::nullopt;
    }
};

ScriptDiagnostic from_location(const std::string& severity, const std::string& kind,
                               const std::string& message, const Luau::Location& location) {
    ScriptDiagnostic d;
    d.severity = severity;
    d.kind = kind;
    d.message = message;
    d.line = static_cast<int>(location.begin.line) + 1;
    d.column = static_cast<int>(location.begin.column) + 1;
    d.end_line = static_cast<int>(location.end.line) + 1;
    d.end_column = static_cast<int>(location.end.column) + 1;
    return d;
}

} // namespace

std::vector<ScriptDiagnostic> analyze_luau(const std::string& source,
                                           const std::string& module_name) {
    // Declare the standard Luau + Roblox globals as `any` so real
    // gameplay scripts are not drowned in unknown-global errors (the
    // checker stays authoritative for everything else). Diagnostics
    // are remapped out of the prelude afterwards.
    static const std::string kPrelude =
        "declare game: any\n"
        "declare workspace: any\n"
        "declare script: any\n"
        "declare Instance: any\n"
        "declare Enum: any\n"
        "declare task: any\n"
        "declare Vector3: any\n"
        "declare Vector2: any\n"
        "declare CFrame: any\n"
        "declare Color3: any\n"
        "declare UDim: any\n"
        "declare UDim2: any\n"
        "declare BrickColor: any\n"
        "declare NumberRange: any\n"
        "declare Rect: any\n"
        "declare RaycastParams: any\n"
        "declare warn: any\n";
    const int prelude_lines = 17;
    // Hot comments (--!strict etc.) are only honoured at the very top
    // of the module: lift leading ones above the prelude.
    std::string hot;
    int hot_lines = 0;
    {
        size_t pos = 0;
        while (pos < source.size()) {
            size_t eol = source.find('\n', pos);
            std::string line = source.substr(pos, eol == std::string::npos ? std::string::npos : eol - pos);
            std::string trimmed = line;
            size_t first = trimmed.find_first_not_of(" \t\r");
            if (first != std::string::npos) trimmed = trimmed.substr(first);
            if (trimmed.rfind("--!", 0) == 0 && hot_lines < 4) {
                hot += line + "\n";
                ++hot_lines;
                pos = eol == std::string::npos ? source.size() : eol + 1;
            } else {
                break;
            }
        }
    }
    const int header_lines = prelude_lines + hot_lines;
    StringFileResolver resolver;
    resolver.module_name = module_name;
    resolver.source = hot + kPrelude + source;
    Luau::NullConfigResolver config_resolver;
    Luau::Frontend frontend(Luau::SolverMode::New, &resolver, &config_resolver);
    // Official builtin globals (print, math, string, task-less Luau
    // standard library) exactly as the Luau CLI analyzer registers
    // them. Roblox globals come from the source prelude above.
    Luau::registerBuiltinGlobals(frontend, frontend.globals);
    Luau::CheckResult result = frontend.check(module_name);

    std::vector<ScriptDiagnostic> out;
    auto remap = [&](ScriptDiagnostic d) {
        d.line -= header_lines;
        d.end_line -= header_lines;
        return d;
    };
    (void)remap;
    for (const Luau::TypeError& error : result.errors) {
        // Syntax errors are reported through the same channel with a
        // dedicated variant; everything else is a type error.
        const bool syntax = Luau::get<Luau::SyntaxError>(error) != nullptr;
        ScriptDiagnostic d = from_location("error", syntax ? "syntax" : "type",
                                           Luau::toString(error), error.location);
        if (d.line > header_lines) out.push_back(remap(d));
    }
    for (const Luau::LintWarning& warning : result.lintResult.warnings) {
        ScriptDiagnostic d = from_location("warning", "lint", warning.text, warning.location);
        if (d.line > header_lines) out.push_back(remap(d));
    }
    for (const Luau::LintWarning& warning : result.lintResult.errors) {
        ScriptDiagnostic d = from_location("error", "lint", warning.text, warning.location);
        if (d.line > header_lines) out.push_back(remap(d));
    }
    return out;
}

} // namespace blox
