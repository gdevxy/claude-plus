use serde_json::{Map, Value, json};

pub const SHRUNK_KEY: &str = "$chars";

pub fn string_chars(value: &Value) -> u64 {
    match value {
        Value::String(s) => s.chars().count() as u64,
        Value::Array(items) => items.iter().map(string_chars).sum(),
        Value::Object(map) => match shrunk_length(map) {
            Some(length) => length,
            None => map.values().map(string_chars).sum(),
        },
        _ => 0,
    }
}

pub fn shrink(value: &Value, max_len: usize) -> Value {
    match value {
        Value::String(s) if s.len() > max_len => json!({ SHRUNK_KEY: s.chars().count() }),
        Value::Array(items) => {
            Value::Array(items.iter().map(|item| shrink(item, max_len)).collect())
        }
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| (key.clone(), shrink(item, max_len)))
                .collect::<Map<_, _>>(),
        ),
        other => other.clone(),
    }
}

fn shrunk_length(map: &Map<String, Value>) -> Option<u64> {
    if map.len() == 1 {
        map.get(SHRUNK_KEY)?.as_u64()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_every_string_leaf() {
        let value = json!({"a": "abc", "b": ["de", {"c": "f"}], "n": 12});
        assert_eq!(string_chars(&value), 6);
    }

    #[test]
    fn shrink_replaces_long_strings_with_their_length() {
        let value = json!({"short": "hi", "long": "x".repeat(10)});
        let shrunk = shrink(&value, 5);
        assert_eq!(shrunk, json!({"short": "hi", "long": {"$chars": 10}}));
        assert_eq!(string_chars(&shrunk), 12);
    }

    #[test]
    fn counts_characters_not_bytes() {
        assert_eq!(string_chars(&json!("héllo")), 5);
    }
}
