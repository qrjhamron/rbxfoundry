#pragma once
// Engine IPC composition root: the JSON-RPC server owns the edit
// DataModel, the runtime DataModel/Runtime, PhysicsWorld and Renderer,
// and dispatches each method family to a per-domain handler (separate
// translation units under src/ipc/). IPC framing lives in server.cpp;
// handlers only translate JSON <-> engine operations.
#include <string>
#include <vector>

#include "blox/core/json.hpp"
#include "blox/datamodel/datamodel.hpp"
#include "blox/physics/physics.hpp"
#include "blox/rendering/renderer.hpp"
#include "blox/scripting/runtime.hpp"

namespace blox::ipc {

inline constexpr const char* kProtocolVersion = "1";
inline constexpr const char* kEngineVersion = "0.2.0-beta.1";

class EngineServer {
public:
    struct Reply {
        bool is_error = false;
        int code = 0;
        std::string message;
        Json result;
        bool shutdown = false;
        static Reply ok(Json r) { Reply x; x.result = std::move(r); return x; }
        static Reply error(int c, std::string m) {
            Reply x; x.is_error = true; x.code = c; x.message = std::move(m); return x;
        }
    };

    Reply handle(const std::string& method, const Json& params);
    // Newline-delimited JSON-RPC loop on stdin/stdout (diagnostics to
    // stderr only). Returns the process exit code.
    int run();

private:
    Reply handle_script(const std::string& method, const Json& params);
    Reply handle_datamodel_newload(const std::string& method, const Json& params);
    Reply handle_datamodel(const std::string& method, const Json& params);
    Reply handle_runtime(const std::string& method, const Json& params);
    Reply handle_debug(const std::string& method, const Json& params);
    Reply handle_physics(const std::string& method, const Json& params);
    Reply handle_terrain(const std::string& method, const Json& params);
    Reply handle_render(const std::string& method, const Json& params);
    Json runtime_state();
    Json character_json();

    DataModel model_;
    DataModel runtime_model_;
    std::unique_ptr<Runtime> runtime_;
    bool runtime_active_ = false;
    std::vector<Json> outgoing_remotes_;
    std::unique_ptr<PhysicsWorld> physics_;
    Renderer renderer_;
    bool initialized_ = false;
    bool has_model_ = false;
};

} // namespace blox::ipc
