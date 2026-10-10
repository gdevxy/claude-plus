use serde_json::{Map, Value, json};

pub const SHRUNK_KEY: &str = "$chars";

pub fn string_chars(value: &Value) -> u64 {
    match value {
        Value::String(s) => s.chars().count() as u64,
        Value::Array(items) => items.iter().map(string_chars).fold(0, u64::saturating_add),
        Value::Object(map) => match shrunk_length(map) {
            Some(length) => length,
            None => map.values().map(string_chars).fold(0, u64::saturating_add),
        },
        _ => 0,
    }
}

pub fn string_chars_except(value: &Value, skipped_keys: &[&str]) -> u64 {
    match value {
        Value::Array(items) => items
            .iter()
            .map(|item| string_chars_except(item, skipped_keys))
            .fold(0, u64::saturating_add),
        Value::Object(map) if shrunk_length(map).is_none() => map
            .iter()
            .filter(|(key, item)| !(skipped_keys.contains(&key.as_str()) && is_label(item)))
            .map(|(_, item)| string_chars_except(item, skipped_keys))
            .fold(0, u64::saturating_add),
        other => string_chars(other),
    }
}

pub fn is_label(value: &Value) -> bool {
    match value {
        Value::String(_) => true,
        Value::Array(items) => items.iter().all(Value::is_string),
        _ => false,
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
    fn skipped_keys_do_not_count_at_any_depth() {
        let value = json!({"type": "x", "body": "abc", "files": [{"path": "/p", "content": {"$chars": 4}}]});
        assert_eq!(string_chars_except(&value, &["type", "path"]), 7);
        let nested = json!({"path": {"description": "abcd"}, "type": ["a", "b"]});
        assert_eq!(string_chars_except(&nested, &["type", "path"]), 4);
    }

    #[test]
    fn counts_characters_not_bytes() {
        assert_eq!(string_chars(&json!("héllo")), 5);
    }
}
