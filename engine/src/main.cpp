// blox-engine entrypoint: application startup and composition only.
// All IPC and engine behaviour lives in blox::ipc::EngineServer
// (engine/src/ipc/); see docs/ARCHITECTURE.md.
#include <iostream>
#include <string>

#include "blox/ipc/server.hpp"

int main(int argc, char** argv) {
    for (int i = 1; i < argc; ++i) {
        std::string arg = argv[i];
        if (arg == "--version") {
            std::cout << "blox-engine " << blox::ipc::kEngineVersion << " (protocol "
                      << blox::ipc::kProtocolVersion << ")\n";
            return 0;
        }
        if (arg == "--help") {
            std::cout << "blox-engine " << blox::ipc::kEngineVersion << "\n"
                      << "Persistent DataModel process. Reads newline-delimited JSON-RPC 2.0\n"
                      << "requests on stdin, writes responses on stdout. Call method\n"
                      << "\"initialize\" with {\"protocolVersion\":\"1\"} first.\n"
                      << "Methods: initialize, ping, datamodel/new, datamodel/load,\n"
                      << "datamodel/snapshot, instance/add, instance/get, instance/set,\n"
                      << "validate, shutdown.\n";
            return 0;
        }
    }
    blox::ipc::EngineServer server;
    return server.run();
}
