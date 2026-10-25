//! Luau code completion for `blox script complete` (Phase 23).
//!
//! Suggestions come from the official Roblox reflection database
//! (rbx_reflection_database, the same metadata the serializers use):
//! class names for `Instance.new("...")`, service names for
//! `game:GetService("...")`, and properties (with types) plus common
//! Instance methods for member access on receivers that resolve to a
//! known class — including `local x = game:GetService("...")` and
//! `local x = Instance.new("...")` bindings earlier in the source.
//! This is a symbol completer, not a type-inference engine; receivers
//! it cannot resolve produce no member suggestions rather than guesses.

use rbx_reflection::ReflectionDatabase;

use crate::tree::Node;

pub struct Completion {
    pub label: String,
    pub kind: &'static str, // "class" | "service" | "property" | "method" | "enum" | "enumItem" | "keyword" | "global"
    pub detail: String,
}

const KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
    "continue", "export", "type",
];

const GLOBALS: &[&str] = &[
    "game", "workspace", "script", "Instance", "Enum", "task", "math", "string", "table",
    "pairs", "ipairs", "print", "warn", "Vector3", "Vector2", "CFrame", "Color3", "UDim",
    "UDim2", "BrickColor", "NumberRange", "Rect", "tick", "typeof", "tostring", "tonumber",
    "select", "pcall", "require", "wait", "spawn", "delay",
];

const INSTANCE_METHODS: &[&str] = &[
    "FindFirstChild", "FindFirstChildOfClass", "WaitForChild", "GetChildren", "GetDescendants",
    "FindFirstAncestor", "Clone", "Destroy", "GetAttribute", "SetAttribute", "GetAttributes",
    "GetPropertyChangedSignal", "IsA", "GetFullName", "ClearAllChildren",
];

fn members_of(db: &ReflectionDatabase, class: &str) -> Vec<Completion> {
    let mut out = Vec::new();
    let mut current = Some(class);
    let mut depth = 0;
    while let Some(name) = current {
        depth += 1;
        if depth > 32 {
            break;
        }
        let Some(desc) = db.classes.get(name) else { break };
        for (prop_name, prop) in &desc.properties {
            out.push(Completion {
                label: prop_name.to_string(),
                kind: "property",
                detail: format!("{} ({})", prop_name, format!("{:?}", prop.data_type.ty())),
            });
        }
        current = desc.superclass;
    }
    for method in INSTANCE_METHODS {
        out.push(Completion {
            label: method.to_string(),
            kind: "method",
            detail: format!("{method} (Instance method)"),
        });
    }
    out
}

/// Extracts (line_prefix_up_to_cursor, current_word_prefix).
fn cursor_context(source: &str, line: usize, column: usize) -> (String, String) {
    let lines: Vec<&str> = source.lines().collect();
    let text = lines.get(line.saturating_sub(1)).copied().unwrap_or("");
    let upto: String = text.chars().take(column.saturating_sub(1)).collect();
    let word: String = upto
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    (upto, word)
}

fn resolve_receiver(source: &str, receiver: &str, script_class: &str, root: &Node) -> Option<String> {
    match receiver {
        "game" => return Some("DataModel".to_string()),
        "workspace" => return Some("Workspace".to_string()),
        "script" => return Some(script_class.to_string()),
        "Enum" => return None, // handled by enum branch
        _ => {}
    }
    // `local x = game:GetService("Class")` / `local x = Instance.new("Class")`
    for pattern in ["GetService(\"", "Instance.new(\""] {
        let needle = format!("{receiver} = ");
        for line in source.lines() {
            let trimmed = line.trim_start_matches("local ").trim();
            if let Some(rest) = trimmed.strip_prefix(&needle) {
                if let Some(pos) = rest.find(pattern) {
                    let after = &rest[pos + pattern.len()..];
                    if let Some(end) = after.find('"') {
                        return Some(after[..end].to_string());
                    }
                }
            }
        }
    }
    // A service name used directly (e.g. `Players:` in client code is a
    // global only in real Roblox; here resolve root children names).
    for child in &root.children {
        if child.name == receiver {
            return Some(child.class_name.clone());
        }
    }
    None
}

pub fn completions(
    source: &str,
    line: usize,
    column: usize,
    script_class: &str,
    root: &Node,
) -> Vec<Completion> {
    let Ok(db) = rbx_reflection_database::get() else {
        return Vec::new();
    };
    let (upto, word) = cursor_context(source, line, column);
    let mut out: Vec<Completion> = Vec::new();
    let matches = |label: &str| label.to_lowercase().starts_with(&word.to_lowercase());

    // Inside Instance.new("...") → class names.
    if let Some(pos) = upto.rfind("Instance.new(\"") {
        if !upto[pos..].contains("\")") {
            for name in db.classes.keys() {
                if matches(name) {
                    out.push(Completion {
                        label: name.to_string(),
                        kind: "class",
                        detail: format!("Instance.new(\"{name}\")"),
                    });
                }
            }
            return out;
        }
    }
    // Inside GetService("...") → services present in this place.
    if let Some(pos) = upto.rfind("GetService(\"") {
        if !upto[pos..].contains("\")") {
            for child in &root.children {
                if matches(&child.name) {
                    out.push(Completion {
                        label: child.name.clone(),
                        kind: "service",
                        detail: format!("{} service", child.class_name),
                    });
                }
            }
            return out;
        }
    }

    let before_word = upto[..upto.len() - word.len()].trim_end();
    if before_word.ends_with('.') || before_word.ends_with(':') {
        // Receiver = identifier before the separator.
        let receiver: String = before_word[..before_word.len() - 1]
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        if receiver == "Enum" {
            for name in db.enums.keys() {
                if matches(name) {
                    out.push(Completion {
                        label: name.to_string(),
                        kind: "enum",
                        detail: format!("Enum.{name}"),
                    });
                }
            }
            return out;
        }
        if let Some(enum_desc) = db.enums.get(receiver.as_str()) {
            for item in enum_desc.items.keys() {
                if matches(item) {
                    out.push(Completion {
                        label: item.to_string(),
                        kind: "enumItem",
                        detail: format!("Enum.{receiver}.{item}"),
                    });
                }
            }
            return out;
        }
        if receiver == "game" {
            // Services are the DataModel's dynamic members.
            for child in &root.children {
                if matches(&child.name) {
                    out.push(Completion {
                        label: child.name.clone(),
                        kind: "service",
                        detail: format!("{} service", child.class_name),
                    });
                }
            }
        }
        if let Some(class) = resolve_receiver(source, &receiver, script_class, root) {
            for c in members_of(&db, &class) {
                if matches(&c.label) {
                    out.push(c);
                }
            }
        }
        return out;
    }

    // Statement position: keywords + globals + service globals.
    for kw in KEYWORDS {
        if matches(kw) {
            out.push(Completion { label: kw.to_string(), kind: "keyword", detail: "Luau keyword".to_string() });
        }
    }
    for g in GLOBALS {
        if matches(g) {
            out.push(Completion { label: g.to_string(), kind: "global", detail: "Roblox/Luau global".to_string() });
        }
    }
    out
}
