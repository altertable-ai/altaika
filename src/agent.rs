use serde::Serialize;
use serde_json::{Value, json};

pub fn agent_response<S, D>(kind: &str, skill: &str, mode: &str, stats: S, data: D) -> Value
where
    S: Serialize,
    D: Serialize,
{
    json!({
        "kind": kind,
        "schema_version": "1.0",
        "skill": skill,
        "mode": mode,
        "stats": stats,
        "data": data,
    })
}
