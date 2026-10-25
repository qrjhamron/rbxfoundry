#include "blox/ipc/server.hpp"
#include "server_internal.hpp"

namespace blox::ipc {

EngineServer::Reply EngineServer::handle_physics(const std::string& method, const Json& params) {
            if (!runtime_active_ || !runtime_) { return Reply::error(-32021, "no runtime session: call 'runtime/begin' first"); }
                        if (method == "physics/begin") {
                std::string rig = "R6";
                if (const Json* r = params.find("rig")) rig = r->as_string();
                physics_ = std::make_unique<blox::PhysicsWorld>(runtime_model_);
                physics_->spawn_character(rig);
                physics_->build_world();
                runtime_->set_physics(physics_.get());
                Json result = Json::object();
                result.set("bodies", Json(static_cast<int64_t>(physics_->body_count())));
                result.set("character", character_json());
                return Reply::ok(std::move(result));
            }
            if (method == "physics/beginMulti") {
                physics_ = std::make_unique<blox::PhysicsWorld>(runtime_model_);
                runtime_->set_physics(physics_.get());
                Json chars = Json::array();
                if (const Json* players = params.find("players")) {
                    if (players->is_array()) {
                        for (const Json& pl : players->as_array()) {
                            std::string pname = pl.find("name") ? pl.find("name")->as_string() : "Player";
                            std::string rig = pl.find("rig") ? pl.find("rig")->as_string() : "R6";
                            int idx = physics_->spawn_character_multi(rig, pname);
                            Json item = Json::object();
                            item.set("index", Json(static_cast<int64_t>(idx)));
                            item.set("name", Json(pname));
                            chars.push(std::move(item));
                        }
                    }
                }
                physics_->build_world();
                runtime_->set_character_resolver([&](Instance* player) -> Instance* {
                    if (!physics_ || !player) return nullptr;
                    for (size_t i = 0; i < physics_->character_count(); ++i) {
                        Instance* m = physics_->character_model_at(i);
                        if (m && m->name == player->name) return m;
                    }
                    return nullptr;
                });
                Json result = Json::object();
                result.set("bodies", Json(static_cast<int64_t>(physics_->body_count())));
                result.set("characters", std::move(chars));
                return Reply::ok(std::move(result));
            }
            if (!physics_) { return Reply::error(-32022, "physics not begun: call 'physics/begin' first"); }
            if (method == "physics/step") {
                double dt = 1.0 / 60.0;
                if (const Json* d = params.find("dt")) dt = d->as_double(dt);
                int steps = 1;
                if (const Json* n = params.find("steps")) steps = static_cast<int>(n->as_int(1));
                std::array<double, 2> move{0, 0};
                bool jump = false;
                double yaw = 0.0;
                if (const Json* input = params.find("input")) {
                    if (const Json* m = input->find("move")) {
                        if (m->is_array() && m->as_array().size() == 2) {
                            move = {m->as_array()[0].as_double(), m->as_array()[1].as_double()};
                        }
                    }
                    if (const Json* j = input->find("jump")) jump = j->as_bool();
                    if (const Json* y = input->find("yaw")) yaw = y->as_double();
                }
                Json events = Json::array();
                for (int i = 0; i < steps; ++i) {
                    physics_->set_character_input(move, jump, yaw);
                    std::vector<Instance*> moved = physics_->step(dt);
                    runtime_->notify_moved(moved);
                    runtime_->step(dt);
                    physics_->set_character_anim_driven(
                        0, runtime_->animation_playing_in(physics_->character_model_at(0)));
                    for (auto& [touched, other] : physics_->take_touch_events()) {
                        Json args = Json::array();
                        Json tag = Json::object();
                        tag.set("$t", Json("Instance"));
                        tag.set("p", Json(runtime_model_.path_of(*other)));
                        args.push(std::move(tag));
                        runtime_->fire_signal_json(touched, "Touched", args.dump());
                    }
                    for (const std::string& event : physics_->take_events()) {
                        events.push(Json(event));
                        if (event == "died" && physics_->humanoid())
                            runtime_->fire_event(physics_->humanoid(), "Died");
                    }
                    if (physics_->humanoid())
                        runtime_->set_humanoid_state(physics_->humanoid(),
                                                    physics_->character_info().state);
                }
                Json result = Json::object();
                result.set("character", character_json());
                result.set("time", Json(physics_->time()));
                result.set("events", std::move(events));
                result.set("threads", Json(static_cast<int64_t>(runtime_->thread_count())));
                Json logs = Json::array();
                for (const auto& entry : runtime_->logs()) {
                    Json item = Json::object();
                    item.set("level", Json(entry.level));
                    item.set("message", Json(entry.message));
                    item.set("source", Json(entry.source));
                    logs.push(std::move(item));
                }
                result.set("logs", std::move(logs));
                Json errors = Json::array();
                for (const auto& entry : runtime_->errors()) {
                    Json item = Json::object();
                    item.set("message", Json(entry.message));
                    item.set("traceback", Json(entry.traceback));
                    item.set("source", Json(entry.source));
                    errors.push(std::move(item));
                }
                result.set("errors", std::move(errors));
                return Reply::ok(std::move(result));
            }
            if (method == "physics/state") {
                Json result = Json::object();
                result.set("character", character_json());
                result.set("time", Json(physics_->time()));
                result.set("bodies", Json(static_cast<int64_t>(physics_->body_count())));
                return Reply::ok(std::move(result));
            }
            if (method == "physics/raycast") {
                auto vec3_param = [&](const char* key, std::array<double, 3>& out) {
                    const Json* v = params.find(key);
                    if (!v || !v->is_array() || v->as_array().size() != 3) return false;
                    out = {v->as_array()[0].as_double(), v->as_array()[1].as_double(), v->as_array()[2].as_double()};
                    return true;
                };
                std::array<double, 3> origin{}, direction{};
                if (!vec3_param("origin", origin) || !vec3_param("direction", direction)) {
                    return Reply::error(-32602, "raycast needs origin and direction [x,y,z]");
                }
                double max_distance = 1000.0;
                if (const Json* m = params.find("maxDistance")) max_distance = m->as_double(max_distance);
                blox::PhysicsWorld::RayHit hit = physics_->raycast(origin, direction, max_distance);
                Json result = Json::object();
                result.set("hit", Json(hit.hit));
                if (hit.hit) {
                    result.set("path", Json(hit.path));
                    Json point = Json::array();
                    for (double d : hit.point) point.push(Json(d));
                    result.set("point", std::move(point));
                    Json normal = Json::array();
                    for (double d : hit.normal) normal.push(Json(d));
                    result.set("normal", std::move(normal));
                    result.set("distance", Json(hit.distance));
                }
                return Reply::ok(std::move(result));
            }
            if (method == "physics/stepMulti") {
                if (!physics_) { return Reply::error(-32022, "physics not begun: call 'physics/beginMulti' first"); }
                double dt = 1.0 / 60.0;
                if (const Json* d = params.find("dt")) dt = d->as_double(dt);
                int steps = 1;
                if (const Json* n = params.find("steps")) steps = static_cast<int>(n->as_int(1));
                const Json* inputs = params.find("inputs");
                for (int i = 0; i < steps; ++i) {
                    if (inputs && inputs->is_array()) {
                        size_t c = 0;
                        for (const Json& input : inputs->as_array()) {
                            std::array<double, 2> move{0, 0};
                            bool jump = false;
                            double yaw = 0.0;
                            if (const Json* m = input.find("move")) {
                                if (m->is_array() && m->as_array().size() == 2)
                                    move = {m->as_array()[0].as_double(), m->as_array()[1].as_double()};
                            }
                            if (const Json* j = input.find("jump")) jump = j->as_bool();
                            if (const Json* y = input.find("yaw")) yaw = y->as_double();
                            physics_->set_character_input_at(c, move, jump, yaw);
                            ++c;
                        }
                    }
                    std::vector<Instance*> moved = physics_->step(dt);
                    runtime_->notify_moved(moved);
                    runtime_->step(dt);
                    for (size_t c = 0; c < physics_->character_count(); ++c)
                        physics_->set_character_anim_driven(
                            c, runtime_->animation_playing_in(physics_->character_model_at(c)));
                    for (size_t c = 0; c < physics_->character_count(); ++c) {
                        for (const std::string& event : physics_->take_events_at(c)) {
                            if (event == "died" && physics_->humanoid_at(c))
                                runtime_->fire_event(physics_->humanoid_at(c), "Died");
                        }
                        if (physics_->humanoid_at(c))
                            runtime_->set_humanoid_state(physics_->humanoid_at(c),
                                                        physics_->character_info_at(c).state);
                    }
                    for (auto& [touched, other] : physics_->take_touch_events()) {
                        Json args = Json::array();
                        Json tag = Json::object();
                        tag.set("$t", Json("Instance"));
                        tag.set("p", Json(runtime_model_.path_of(*other)));
                        args.push(std::move(tag));
                        runtime_->fire_signal_json(touched, "Touched", args.dump());
                    }
                }
                Json result = Json::object();
                Json chars = Json::array();
                for (size_t c = 0; c < physics_->character_count(); ++c) {
                    blox::PhysicsWorld::CharacterInfo info = physics_->character_info_at(c);
                    Json item = Json::object();
                    item.set("present", Json(info.present));
                    Json pos = Json::array();
                    for (double d : info.position) pos.push(Json(d));
                    item.set("position", std::move(pos));
                    Json vel = Json::array();
                    for (double d : info.velocity) vel.push(Json(d));
                    item.set("velocity", std::move(vel));
                    item.set("grounded", Json(info.grounded));
                    item.set("health", Json(info.health));
                    item.set("state", Json(info.state));
                    item.set("yaw", Json(info.yaw));
                    if (Instance* m = physics_->character_model_at(c)) item.set("name", Json(m->name));
                    chars.push(std::move(item));
                }
                result.set("characters", std::move(chars));
                result.set("time", Json(physics_->time()));
                Json logs = Json::array();
                for (const auto& entry : runtime_->logs()) {
                    Json item = Json::object();
                    item.set("level", Json(entry.level));
                    item.set("message", Json(entry.message));
                    item.set("source", Json(entry.source));
                    logs.push(std::move(item));
                }
                result.set("logs", std::move(logs));
                Json errors = Json::array();
                for (const auto& entry : runtime_->errors()) {
                    Json item = Json::object();
                    item.set("message", Json(entry.message));
                    item.set("traceback", Json(entry.traceback));
                    item.set("source", Json(entry.source));
                    errors.push(std::move(item));
                }
                result.set("errors", std::move(errors));
                Json calls = Json::array();
                for (auto& c : outgoing_remotes_) calls.push(std::move(c));
                outgoing_remotes_.clear();
                result.set("remoteCalls", std::move(calls));
                return Reply::ok(std::move(result));
            }
            if (method == "physics/queryBox" || method == "physics/queryRadius") {
                auto vec3_param = [&](const char* key, std::array<double, 3>& out) {
                    const Json* v = params.find(key);
                    if (!v || !v->is_array() || v->as_array().size() != 3) return false;
                    out = {v->as_array()[0].as_double(), v->as_array()[1].as_double(), v->as_array()[2].as_double()};
                    return true;
                };
                std::array<double, 3> center{};
                if (!vec3_param("center", center)) {
                    return Reply::error(-32602, "query needs center [x,y,z]");
                }
                std::vector<Instance*> hits;
                if (method == "physics/queryBox") {
                    std::array<double, 3> size{1, 1, 1};
                    if (!vec3_param("size", size)) { return Reply::error(-32602, "queryBox needs size [x,y,z]"); }
                    hits = physics_->parts_in_box(center, size);
                } else {
                    double radius = 5.0;
                    if (const Json* r = params.find("radius")) radius = r->as_double(radius);
                    hits = physics_->parts_in_radius(center, radius);
                }
                Json result = Json::object();
                Json paths = Json::array();
                for (Instance* inst : hits) paths.push(Json(runtime_model_.path_of(*inst)));
                result.set("paths", std::move(paths));
                return Reply::ok(std::move(result));
            }
            if (method == "physics/end") {
                physics_.reset();
                Json result = Json::object();
                result.set("active", Json(false));
                return Reply::ok(std::move(result));
            }
            return Reply::error(-32601, "unknown physics method '" + method + "'");

}

} // namespace blox::ipc
