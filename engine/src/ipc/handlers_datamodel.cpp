#include "blox/ipc/server.hpp"
#include "server_internal.hpp"

namespace blox::ipc {

EngineServer::Reply EngineServer::handle_datamodel(const std::string& method, const Json& params) {
        if (method == "datamodel/snapshot") {
            Json result = Json::object();
            result.set("tree", model_.snapshot());
            result.set("instanceCount", Json(static_cast<int64_t>(model_.instance_count())));
            return Reply::ok(std::move(result));
        } else if (method == "instance/add") {
            std::string parent_path, class_name, name, error;
            if (!require_string(params, "parent", parent_path, error) ||
                !require_string(params, "className", class_name, error)) { return Reply::error(-32602, error); }
            if (class_name.empty()) { return Reply::error(-32602, "className must not be empty"); }
            if (const Json* n = params.find("name")) name = n->as_string();
            Instance* parent = model_.resolve_path(parent_path, error);
            if (!parent) { return Reply::error(-32004, error); }
            Instance& inst = model_.add_instance(*parent, class_name, name);
            Json result = Json::object();
            result.set("path", Json(model_.path_of(inst)));
            result.set("instance", DataModel::instance_to_json(inst));
            return Reply::ok(std::move(result));
        } else if (method == "instance/get") {
            std::string target, error;
            if (!require_string(params, "target", target, error)) { return Reply::error(-32602, error); }
            Instance* inst = model_.resolve_path(target, error);
            if (!inst) { return Reply::error(-32004, error); }
            Json result = Json::object();
            result.set("path", Json(model_.path_of(*inst)));
            result.set("instance", DataModel::instance_to_json(*inst));
            return Reply::ok(std::move(result));
        } else if (method == "instance/set") {
            std::string target, property, error;
            if (!require_string(params, "target", target, error) ||
                !require_string(params, "property", property, error)) { return Reply::error(-32602, error); }
            const Json* value_j = params.find("value");
            if (!value_j) { return Reply::error(-32602, "missing 'value' parameter"); }
            Variant value;
            if (!Variant::from_json(*value_j, value, error)) { return Reply::error(-32602, error); }
            Instance* inst = model_.resolve_path(target, error);
            if (!inst) { return Reply::error(-32004, error); }
            if (!model_.set_property(*inst, property, value, error)) { return Reply::error(-32005, error); }
            Json result = Json::object();
            result.set("path", Json(model_.path_of(*inst)));
            result.set("instance", DataModel::instance_to_json(*inst));
            return Reply::ok(std::move(result));
        } else if (method == "instance/clone") {
            std::string target, error;
            if (!require_string(params, "target", target, error)) { return Reply::error(-32602, error); }
            Instance* inst = model_.resolve_path(target, error);
            if (!inst) { return Reply::error(-32004, error); }
            Instance* parent = inst->parent;
            std::string parent_path;
            if (require_string(params, "parent", parent_path, error)) {
                parent = model_.resolve_path(parent_path, error);
                if (!parent) { return Reply::error(-32004, error); }
            }
            if (!parent) { return Reply::error(-32004, "cannot clone the DataModel root"); }
            std::string name;
            if (const Json* n = params.find("name")) name = n->as_string();
            Instance& copy = model_.clone_instance(*inst, *parent, name);
            Json result = brief_instance(model_, copy);
            result.set("instance", DataModel::instance_to_json(copy));
            return Reply::ok(std::move(result));
        } else if (method == "instance/delete") {
            std::string target, error;
            if (!require_string(params, "target", target, error)) { return Reply::error(-32602, error); }
            Instance* inst = model_.resolve_path(target, error);
            if (!inst) { return Reply::error(-32004, error); }
            if (inst == &model_.root()) { return Reply::error(-32004, "cannot delete the DataModel root"); }
            std::string path = model_.path_of(*inst);
            size_t removed = model_.delete_instance(*inst);
            Json result = Json::object();
            result.set("path", Json(path));
            result.set("deletedCount", Json(static_cast<int64_t>(removed)));
            result.set("instanceCount", Json(static_cast<int64_t>(model_.instance_count())));
            return Reply::ok(std::move(result));
        } else if (method == "instance/reparent") {
            std::string target, parent_path, error;
            if (!require_string(params, "target", target, error) ||
                !require_string(params, "parent", parent_path, error)) { return Reply::error(-32602, error); }
            Instance* inst = model_.resolve_path(target, error);
            if (!inst) { return Reply::error(-32004, error); }
            Instance* parent = model_.resolve_path(parent_path, error);
            if (!parent) { return Reply::error(-32004, error); }
            if (!model_.reparent(*inst, *parent, error)) { return Reply::error(-32005, error); }
            Json result = brief_instance(model_, *inst);
            return Reply::ok(std::move(result));
        } else if (method == "instance/group") {
            const Json* targets_j = params.find("targets");
            if (!targets_j || !targets_j->is_array() || targets_j->as_array().empty()) {
                return Reply::error(-32602, "'targets' must be a non-empty array of paths");
            }
            std::vector<Instance*> targets;
            std::string error;
            bool ok = true;
            for (const auto& t : targets_j->as_array()) {
                Instance* inst = model_.resolve_path(t.as_string(), error);
                if (!inst) { ok = false; break; }
                if (inst == &model_.root()) { error = "cannot group the DataModel root"; ok = false; break; }
                targets.push_back(inst);
            }
            if (!ok) { return Reply::error(-32004, error); }
            for (size_t i = 0; i < targets.size() && ok; ++i)
                for (size_t j = 0; j < targets.size(); ++j)
                    if (i != j) {
                        Instance* a = targets[i];
                        bool descendant = false;
                        for (Instance* cur = targets[j]->parent; cur; cur = cur->parent)
                            if (cur == a) descendant = true;
                        if (descendant) { error = "targets cannot contain both an instance and its descendant"; ok = false; break; }
                    }
            if (!ok) { return Reply::error(-32005, error); }
            Instance* parent = targets.front()->parent;
            std::string parent_path;
            if (require_string(params, "parent", parent_path, error)) {
                parent = model_.resolve_path(parent_path, error);
                if (!parent) { return Reply::error(-32004, error); }
            }
            std::string name = "Model";
            if (const Json* n = params.find("name")) name = n->as_string();
            Instance& group = model_.group_instances(targets, *parent, name);
            Json result = brief_instance(model_, group);
            result.set("groupedCount", Json(static_cast<int64_t>(targets.size())));
            return Reply::ok(std::move(result));
        } else if (method == "instance/find") {
            DataModel::FindCriteria criteria;
            if (const Json* n = params.find("name")) criteria.name = n->as_string();
            if (const Json* c = params.find("className")) criteria.class_name = c->as_string();
            if (const Json* s = params.find("substring")) criteria.substring = s->as_bool();
            Instance* within = nullptr;
            std::string within_path, error;
            if (require_string(params, "root", within_path, error)) {
                within = model_.resolve_path(within_path, error);
                if (!within) { return Reply::error(-32004, error); }
            }
            auto found = model_.find(criteria, within);
            Json arr = Json::array();
            for (Instance* inst : found) arr.push(brief_instance(model_, *inst));
            Json result = Json::object();
            result.set("matches", std::move(arr));
            result.set("count", Json(static_cast<int64_t>(found.size())));
            return Reply::ok(std::move(result));
        } else if (method == "instance/move" || method == "instance/rotate" || method == "instance/scale") {
            std::string target, error;
            if (!require_string(params, "target", target, error)) { return Reply::error(-32602, error); }
            Instance* inst = model_.resolve_path(target, error);
            if (!inst) { return Reply::error(-32004, error); }
            bool ok = false;
            if (method == "instance/move") {
                std::array<double, 3> delta{};
                if (!require_vec3(params, "delta", delta, error)) { return Reply::error(-32602, error); }
                bool relative = false;
                if (const Json* r = params.find("relative")) relative = r->as_bool();
                ok = model_.move(*inst, delta, relative, error);
            } else if (method == "instance/rotate") {
                std::array<double, 3> angles{};
                if (!require_vec3(params, "angles", angles, error)) { return Reply::error(-32602, error); }
                bool world = false;
                if (const Json* w = params.find("worldSpace")) world = w->as_bool();
                ok = model_.rotate(*inst, angles, world, error);
            } else {
                std::array<double, 3> factors{1, 1, 1};
                if (const Json* f = params.find("factor")) {
                    double v = f->as_double();
                    factors = {v, v, v};
                } else if (!require_vec3(params, "factors", factors, error)) { return Reply::error(-32602, error); }
                ok = model_.scale(*inst, factors, error);
            }
            if (!ok) { return Reply::error(-32005, error); }
            Json result = brief_instance(model_, *inst);
            result.set("instance", DataModel::instance_to_json(*inst));
            return Reply::ok(std::move(result));
        } else if (method == "instance/pivot") {
            std::string target, error;
            if (!require_string(params, "target", target, error)) { return Reply::error(-32602, error); }
            Instance* inst = model_.resolve_path(target, error);
            if (!inst) { return Reply::error(-32004, error); }
            if (inst->class_name != "Model") { return Reply::error(-32005, "pivot only applies to a Model, got " + inst->class_name); }
            const Json* value_j = params.find("value");
            if (value_j) {
                Variant pivot;
                if (!Variant::from_json(*value_j, pivot, error)) { return Reply::error(-32602, error); }
                if (!model_.set_model_pivot(*inst, pivot, error)) { return Reply::error(-32005, error); }
            }
            Json result = brief_instance(model_, *inst);
            result.set("pivot", model_.model_pivot(*inst).to_json());
            return Reply::ok(std::move(result));
        }

    return Reply::error(-32601, "method not found: " + method);
}

} // namespace blox::ipc
