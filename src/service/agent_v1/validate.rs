//! Coerce-then-validate MCP arguments against `src/tools.json`.
//!
//! MCP clients inconsistently emit stringified scalars (`"true"`, `"12"`). We
//! first walk the value and promote stringified scalars where the schema says
//! they should be typed, then run JSON Schema validation. On failure, return a
//! [`ToolError::InvalidArgument`] whose detail includes the JSON Pointer of the
//! offending field.

use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::OnceLock;

use jsonschema::Validator;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use crate::service::helpers::{ToolError, ToolErrorCategory};

/// Proof that a value has been coerce-then-validated against the registered
/// `tools.json` schema for its tool, and successfully deserialized into `T`.
///
/// Handlers should consume `ValidatedArgs<T>` instead of calling `serde_json::from_value`
/// on the raw envelope. Construct via [`ValidatedArgs::from_validated`] (when the
/// dispatcher already ran [`coerce_and_validate`]) or [`ValidatedArgs::take`]
/// (validates inline — useful in tests and adapters).
pub struct ValidatedArgs<T> {
    inner: T,
    _marker: PhantomData<fn() -> T>,
}

impl<T> ValidatedArgs<T> {
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T: DeserializeOwned> ValidatedArgs<T> {
    /// Deserialize a value that the dispatcher already coerced + schema-validated.
    ///
    /// Failures here are not schema violations (those are caught upstream); they
    /// are typed-deserialization mismatches between `tools.json` and the Rust
    /// arg struct, which are operator-facing bugs reported as
    /// `InvalidArgument` so MCP clients see a categorized error.
    pub fn from_validated(value: Value) -> Result<Self, ToolError> {
        match serde_json::from_value::<T>(value) {
            Ok(inner) => Ok(Self {
                inner,
                _marker: PhantomData,
            }),
            Err(error) => Err(ToolError::new(
                ToolErrorCategory::InvalidArgument,
                "INVALID_ARGUMENT",
                format!("failed to deserialize validated arguments: {error}"),
                json!({ "error": error.to_string() }),
            )),
        }
    }

    /// Coerce + validate against the schema for `tool`, then deserialize.
    #[allow(dead_code)]
    pub fn take(tool: &str, value: Value) -> Result<Self, ToolError> {
        let validated = coerce_and_validate(tool, value)?;
        Self::from_validated(validated)
    }
}

const TOOLS_JSON: &str = include_str!("../../tools.json");

struct ToolSchema {
    schema: Value,
    validator: Validator,
}

struct ToolRegistry {
    schemas: HashMap<String, ToolSchema>,
    definitions: Vec<Value>,
}

static REGISTRY: OnceLock<Result<ToolRegistry, String>> = OnceLock::new();

fn registry() -> &'static Result<ToolRegistry, String> {
    REGISTRY.get_or_init(build_registry)
}

fn schemas() -> Option<&'static HashMap<String, ToolSchema>> {
    registry().as_ref().ok().map(|r| &r.schemas)
}

/// Force schema initialization and surface any load error as a fatal startup
/// error. Called from `CodeService::new`; a corrupt or reshaped `tools.json`
/// aborts startup cleanly instead of panicking on first validated call.
pub fn init_schemas() -> anyhow::Result<()> {
    match registry() {
        Ok(_) => Ok(()),
        Err(err) => Err(anyhow::anyhow!("{err}")),
    }
}

/// Parsed `tools.json` array, ready to hand to `tools/list`. Empty slice if
/// [`init_schemas`] has not run or failed — the MCP entry point guarantees
/// init runs first, so production callers see the full list.
pub fn tool_definitions() -> &'static [Value] {
    registry()
        .as_ref()
        .map(|r| r.definitions.as_slice())
        .unwrap_or(&[])
}

fn build_registry() -> Result<ToolRegistry, String> {
    let parsed: Value = serde_json::from_str(TOOLS_JSON)
        .map_err(|e| format!("src/tools.json failed to parse: {e}"))?;
    let Value::Array(tools) = parsed else {
        return Err("src/tools.json root must be a JSON array".to_string());
    };
    let mut schemas = HashMap::with_capacity(tools.len());
    for tool in &tools {
        let Some(name) = tool.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some(schema) = tool.get("inputSchema").cloned() else {
            continue;
        };
        match jsonschema::validator_for(&schema) {
            Ok(validator) => {
                schemas.insert(name.to_string(), ToolSchema { schema, validator });
            }
            Err(error) => {
                tracing::warn!(
                    tool = name,
                    error = %error,
                    "tools.json schema failed to compile; tool will skip validation"
                );
            }
        }
    }
    Ok(ToolRegistry {
        schemas,
        definitions: tools,
    })
}

/// Coerce and validate `arguments` against the registered schema for `tool`.
///
/// Returns the (possibly coerced) arguments value on success. If no schema is
/// registered for the tool, the arguments pass through unchanged — this keeps
/// unregistered or internal tools from being blocked by validation.
pub fn coerce_and_validate(tool: &str, mut arguments: Value) -> Result<Value, ToolError> {
    let Some(tool_schema) = schemas().and_then(|s| s.get(tool)) else {
        return Ok(arguments);
    };
    expand_shorthand_refs(&mut arguments);
    coerce(&mut arguments, &tool_schema.schema);
    if let Err(error) = tool_schema.validator.validate(&arguments) {
        let pointer = error.instance_path.to_string();
        let reason = error.to_string();
        return Err(ToolError::new(
            ToolErrorCategory::InvalidArgument,
            "INVALID_ARGUMENT",
            format!("invalid argument for tool '{tool}' at '{pointer}': {reason}"),
            json!({
                "tool": tool,
                "pointer": pointer,
                "reason": reason,
            }),
        ));
    }
    Ok(arguments)
}

// ── Coercion ────────────────────────────────────────────────────────────────

fn schema_types(schema: &Value) -> Vec<&str> {
    match schema.get("type") {
        Some(Value::String(s)) => vec![s.as_str()],
        Some(Value::Array(arr)) => arr.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

fn coerce(value: &mut Value, schema: &Value) {
    if !schema.is_object() {
        return;
    }

    let types = schema_types(schema);

    // Scalar promotion from string.
    if let Value::String(s) = value {
        if types.contains(&"boolean") && !types.contains(&"string") {
            match s.as_str() {
                "true" => *value = Value::Bool(true),
                "false" => *value = Value::Bool(false),
                _ => {}
            }
        } else if (types.contains(&"integer") || types.contains(&"number"))
            && !types.contains(&"string")
        {
            if let Ok(n) = s.parse::<i64>() {
                *value = Value::from(n);
            } else if let Ok(n) = s.parse::<f64>()
                && let Some(num) = serde_json::Number::from_f64(n)
            {
                *value = Value::Number(num);
            }
        }
    }

    // Recurse into objects.
    if let Value::Object(map) = value {
        if let Some(Value::Object(props)) = schema.get("properties") {
            coerce_object(map, props);
        }
        if let Some(additional) = schema.get("additionalProperties")
            && additional.is_object()
        {
            for (_, v) in map.iter_mut() {
                coerce(v, additional);
            }
        }
    }

    // Recurse into arrays.
    if let (Value::Array(arr), Some(items)) = (&mut *value, schema.get("items"))
        && items.is_object()
    {
        for v in arr.iter_mut() {
            coerce(v, items);
        }
    }
}

fn coerce_object(map: &mut Map<String, Value>, props: &Map<String, Value>) {
    for (key, inner) in map.iter_mut() {
        if let Some(inner_schema) = props.get(key) {
            coerce(inner, inner_schema);
        }
    }
}

/// Expand shorthand ref strings inside a `refs` array.
///
/// - `"path/to/file.rs:40-120"` → `{kind: "file", path, start_line, end_line}`
/// - `"path/to/file.rs:40"` → `{kind: "file", path, start_line, end_line=start_line}`
/// - `"path/to/file.rs::Foo.bar#function"` → `{kind: "symbol", path, stable_id}`
/// - Anything else is left untouched so the validator can report it.
fn expand_shorthand_refs(arguments: &mut Value) {
    if let Value::Object(map) = arguments
        && let Some(Value::Array(arr)) = map.get_mut("refs")
    {
        for v in arr.iter_mut() {
            if let Value::String(s) = v
                && let Some(obj) = shorthand_ref_to_object(s)
            {
                *v = obj;
            }
        }
    }
}

fn shorthand_ref_to_object(s: &str) -> Option<Value> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some((path, rest)) = trimmed.rsplit_once("::")
        && !path.is_empty()
        && !rest.is_empty()
    {
        return Some(json!({
            "kind": "symbol",
            "path": path,
            "stable_id": trimmed,
        }));
    }
    if let Some((path, range)) = trimmed.rsplit_once(':')
        && !path.is_empty()
    {
        // Reject Windows drive letters: `C:\...` has range that isn't numeric.
        let (start, end) = if let Some((a, b)) = range.split_once('-') {
            (a.parse::<u32>().ok()?, b.parse::<u32>().ok()?)
        } else {
            let n = range.parse::<u32>().ok()?;
            (n, n)
        };
        return Some(json!({
            "kind": "file",
            "path": path,
            "start_line": start,
            "end_line": end,
        }));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coerces_stringified_boolean() {
        let schema = json!({
            "type": "object",
            "properties": {
                "incremental": { "type": "boolean" }
            }
        });
        let mut args = json!({ "incremental": "true" });
        coerce(&mut args, &schema);
        assert_eq!(args, json!({ "incremental": true }));
    }

    #[test]
    fn coerces_stringified_integer() {
        let schema = json!({
            "type": "object",
            "properties": {
                "limit": { "type": "integer" }
            }
        });
        let mut args = json!({ "limit": "42" });
        coerce(&mut args, &schema);
        assert_eq!(args, json!({ "limit": 42 }));
    }

    #[test]
    fn rejects_missing_required_field() {
        // search_catalog requires source_id + query.
        let err = coerce_and_validate("search_catalog", json!({ "query": "hi" })).unwrap_err();
        assert_eq!(err.category, ToolErrorCategory::InvalidArgument);
    }

    #[test]
    fn allows_well_formed_args() {
        let ok = coerce_and_validate(
            "search_catalog",
            json!({ "source_id": "repo-abc", "query": "needle" }),
        )
        .unwrap();
        assert_eq!(ok["source_id"], "repo-abc");
    }

    #[test]
    fn tool_without_schema_passes_through() {
        let v = coerce_and_validate("nonexistent_tool", json!({ "x": 1 })).unwrap();
        assert_eq!(v, json!({ "x": 1 }));
    }

    #[test]
    fn shorthand_range_ref_expands() {
        let got = shorthand_ref_to_object("src/foo.rs:40-120").unwrap();
        assert_eq!(got["kind"], "file");
        assert_eq!(got["path"], "src/foo.rs");
        assert_eq!(got["start_line"], 40);
        assert_eq!(got["end_line"], 120);
    }

    #[test]
    fn shorthand_single_line_ref_expands() {
        let got = shorthand_ref_to_object("src/foo.rs:42").unwrap();
        assert_eq!(got["start_line"], 42);
        assert_eq!(got["end_line"], 42);
    }

    #[test]
    fn shorthand_symbol_ref_expands() {
        let got = shorthand_ref_to_object("src/foo.rs::Foo.bar#function").unwrap();
        assert_eq!(got["kind"], "symbol");
        assert_eq!(got["path"], "src/foo.rs");
        assert_eq!(got["stable_id"], "src/foo.rs::Foo.bar#function");
    }

    #[test]
    fn shorthand_invalid_returns_none() {
        assert!(shorthand_ref_to_object("").is_none());
        assert!(shorthand_ref_to_object("just-a-path.rs").is_none());
        assert!(shorthand_ref_to_object("C:\\windows\\file.rs").is_none());
    }

    #[test]
    fn read_content_accepts_string_refs() {
        let out = coerce_and_validate(
            "read_content",
            json!({
                "source_id": "r",
                "refs": ["src/foo.rs:40-120", "src/bar.rs::X#function"],
            }),
        )
        .unwrap();
        let refs = out["refs"].as_array().unwrap();
        assert_eq!(refs[0]["kind"], "file");
        assert_eq!(refs[0]["start_line"], 40);
        assert_eq!(refs[1]["kind"], "symbol");
        assert_eq!(refs[1]["stable_id"], "src/bar.rs::X#function");
    }
}
