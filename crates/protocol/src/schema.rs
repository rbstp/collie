use schemars::schema_for;
use serde_json::Value;

use crate::{ClientFrame, ServerFrame};

pub fn client_frame_schema() -> Value {
    serde_json::to_value(schema_for!(ClientFrame)).expect("schema serializes")
}

pub fn server_frame_schema() -> Value {
    serde_json::to_value(schema_for!(ServerFrame)).expect("schema serializes")
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    fn collect_refs(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(map) => {
                if let Some(Value::String(r)) = map.get("$ref") {
                    out.push(r.clone());
                }
                map.values().for_each(|v| collect_refs(v, out));
            }
            Value::Array(items) => items.iter().for_each(|v| collect_refs(v, out)),
            _ => {}
        }
    }

    fn check(name: &str, schema: Value) {
        let current = serde_json::to_string_pretty(&schema).unwrap() + "\n";
        let path = format!("{}/../../docs/protocol/{name}", env!("CARGO_MANIFEST_DIR"));
        let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(current == on_disk, "{path} is stale; run `just schema`");
        let mut refs = Vec::new();
        collect_refs(&schema, &mut refs);
        assert!(!refs.is_empty());
        for r in refs {
            let pointer = r.strip_prefix('#').expect("local ref");
            assert!(schema.pointer(pointer).is_some(), "{name}: dangling {r}");
        }
    }

    #[test]
    fn checked_in_schemas_are_current_and_resolve() {
        check("client-frame.schema.json", super::client_frame_schema());
        check("server-frame.schema.json", super::server_frame_schema());
    }
}
