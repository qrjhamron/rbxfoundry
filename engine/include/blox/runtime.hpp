#pragma once
#include <memory>
#include <string>
#include <vector>

#include <functional>

#include "blox/datamodel.hpp"

namespace blox {
class PhysicsWorld;
}

struct lua_State;

namespace blox {

// Real Luau (official luau-lang/luau) runtime executing Script,
// LocalScript and ModuleScript instances against a runtime DataModel.
// The runtime model is a separate copy of the edit-mode model: scripts
// never mutate edit state unless the caller explicitly commits the
// runtime snapshot.
class Runtime {
public:
    struct LogEntry {
        std::string level;  // "print" | "warn"
        std::string message;
        std::string source; // script path
    };
    struct ErrorEntry {
        std::string message;
        std::string traceback;
        std::string source; // script path
    };

    explicit Runtime(DataModel& model);
    ~Runtime();
    Runtime(const Runtime&) = delete;
    Runtime& operator=(const Runtime&) = delete;

    // Starts every runnable script (server Scripts first, then
    // LocalScripts). Returns false only if the VM itself failed.
    bool start_scripts();
    // Evaluates a chunk in a fresh sandboxed thread (agent/test entry
    // point). Returns false with `error` on compile/runtime failure.
    bool eval(const std::string& source, const std::string& chunk_name, std::string& error,
              std::string& result_json);
    // Advances simulated time by dt seconds (fixed-step friendly):
    // fires RunService.Stepped, resumes due task threads, advances
    // tweens, fires RunService.Heartbeat.
    void step(double dt);
    double time() const { return time_; }
    bool any_thread_alive() const;
    size_t thread_count() const;

    const std::vector<LogEntry>& logs() const { return logs_; }
    const std::vector<ErrorEntry>& errors() const { return errors_; }
    void clear_logs() { logs_.clear(); }

    DataModel& model() { return model_; }

    // Bridges for the physics/play layer: fire an instance signal if a
    // script connected it, report externally-driven CFrame changes, and
    // feed UserInputService key state.
    void fire_event(Instance* inst, const std::string& signal_key);
    void notify_moved(const std::vector<Instance*>& moved);
    void set_key_state(const std::string& key_name, bool down);

    // ---- Networking (Phase 9) ----
    enum class NetContext { Single, Server, Client };
    // kind: 0 = FireServer (to server), 1 = FireClient (args[0] is the
    // target Player instance), 2 = FireAllClients, 3 = InvokeServer
    // (invoke_id identifies the pending call), 4 = InvokeClient.
    struct RemoteCall {
        Instance* remote = nullptr;
        int kind = 0;
        int from_player = -1;
        int invoke_id = 0;
        std::string args_json; // JSON array of marshalled arguments
    };
    void set_net_context(NetContext context, int player_index);
    NetContext net_context() const;
    int player_index() const;
    void set_local_player(Instance* player);
    void set_remote_sink(std::function<void(const RemoteCall&)> sink);
    void set_character_resolver(std::function<Instance*(Instance*)> resolver);
    // Fires OnServerEvent (server) / OnClientEvent (client) on a remote.
    // `from_player` is only meaningful server-side (may be null).
    void deliver_remote(Instance* remote, Instance* from_player, const std::string& args_json);
    // Delivers a RemoteFunction invocation to this runtime's handler and
    // returns the marshalled return value.
    std::string invoke_remote_handler(Instance* remote, Instance* from_player,
                                      const std::string& args_json, bool& ok);
    // Completes a pending InvokeServer/InvokeClient in this runtime.
    void complete_invoke(int invoke_id, const std::string& result_json, bool ok,
                         const std::string& error);
    // Fires an arbitrary instance signal with marshalled arguments.
    void fire_signal_json(Instance* inst, const std::string& key, const std::string& args_json);

    // Physics world used by Workspace:Raycast / spatial queries (Gate 10).
    void set_physics(PhysicsWorld* physics);
    // True while any AnimationTrack loaded through an Animator inside
    // `model_root` is playing (Gate 10 joint animation).
    bool animation_playing_in(Instance* model_root);
    // Humanoid state bridge (Phase 15): physics reports the simulated
    // state; GetState reads it and StateChanged fires on transitions.
    void set_humanoid_state(Instance* humanoid, const std::string& state);

    // Implementation state, exposed for the C binding helpers in
    // runtime.cpp (this is an internal engine class, not a public API).
    struct Impl;
    std::unique_ptr<Impl> impl_;

    // Internal plumbing shared with the C bindings (runtime.cpp).
    lua_State* state() { return L_; }
    void log(std::string level, std::string message, std::string source);
    void report_error(std::string message, std::string traceback, std::string source);
    static Runtime* from_state(lua_State* L);

private:
    DataModel& model_;
    lua_State* L_ = nullptr;
    double time_ = 0.0;
    std::vector<LogEntry> logs_;
    std::vector<ErrorEntry> errors_;
};

} // namespace blox
