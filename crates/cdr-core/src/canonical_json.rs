use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CanonicalJsonError {
    #[error("could not convert value to JSON: {0}")]
    Encode(#[from] serde_json::Error),
}

pub fn to_canonical_json<T: Serialize>(
    value: &T,
    exclude: &[&str],
) -> Result<String, CanonicalJsonError> {
    let mut json = serde_json::to_value(value)?;
    if let Value::Object(object) = &mut json {
        for key in exclude {
            object.remove(*key);
        }
    }
    serde_json::to_string(&sort_value(json)).map_err(Into::into)
}

fn sort_value(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(sort_value).collect()),
        Value::Object(values) => {
            let sorted: BTreeMap<_, _> = values
                .into_iter()
                .map(|(key, value)| (key, sort_value(value)))
                .collect();
            Value::Object(Map::from_iter(sorted))
        }
        scalar => scalar,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::to_canonical_json;

    #[test]
    fn sorts_nested_keys_and_removes_top_level_exclusions() {
        let value = json!({"z": {"b": 2, "a": 1}, "secret": 3, "a": "한글"});
        let actual = to_canonical_json(&value, &["secret"]).expect("canonical JSON");
        assert_eq!(actual, r#"{"a":"한글","z":{"a":1,"b":2}}"#);
    }
}
