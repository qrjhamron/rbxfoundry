#include "blox/ipc/server.hpp"
#include "server_internal.hpp"

namespace blox::ipc {

EngineServer::Reply EngineServer::handle_datamodel_newload(const std::string& method, const Json& params) {
        if (method == "datamodel/new") {
            std::string name = "Place";
            if (const Json* n = params.find("name")) name = n->as_string();
            model_.reset(name);
            // Optional offline place metadata (never invented by the
            // engine: callers pass explicit values, defaults are the
            // unpublished zeros Roblox reports for local files).
            {
                std::string ignored;
                if (const Json* v = params.find("placeId"))
                    model_.set_property(model_.root(), "PlaceId", Variant::make_int64(v->as_int()), ignored);
                if (const Json* v = params.find("gameId"))
                    model_.set_property(model_.root(), "GameId", Variant::make_string(v->as_string()), ignored);
                if (const Json* v = params.find("placeVersion"))
                    model_.set_property(model_.root(), "PlaceVersion", Variant::make_int64(v->as_int()), ignored);
                if (const Json* v = params.find("jobId"))
                    model_.set_property(model_.root(), "JobId", Variant::make_string(v->as_string()), ignored);
            }
            has_model_ = true;
            Json result = Json::object();
            result.set("tree", model_.snapshot());
            return Reply::ok(std::move(result));
        }
        if (method == "datamodel/load") {
            const Json* tree = params.find("tree");
            if (!tree) { return Reply::error(-32602, "missing 'tree' parameter"); }
            std::string error;
            if (!model_.load_snapshot(*tree, error)) { return Reply::error(-32002, error); }
            has_model_ = true;
            Json result = Json::object();
            result.set("instanceCount", Json(static_cast<int64_t>(model_.instance_count())));
            return Reply::ok(std::move(result));
        }

    return Reply::error(-32601, "method not found: " + method);
}

} // namespace blox::ipc
