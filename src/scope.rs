use std::collections::HashSet;

use anyhow::Result;

use crate::store::Store;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    All,
    Account(String),
}

impl Scope {
    pub fn account(&self) -> Option<&str> {
        match self {
            Scope::All => None,
            Scope::Account(id) => Some(id),
        }
    }

    pub fn sessions(&self, store: &Store) -> Result<Option<HashSet<String>>> {
        match self {
            Scope::All => Ok(None),
            Scope::Account(id) => store.sessions_of(id).map(Some),
        }
    }

    pub fn keeps(&self, sessions: &Option<HashSet<String>>, session: &str) -> bool {
        sessions.as_ref().is_none_or(|kept| kept.contains(session))
    }
}

pub fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = bytes.get(index + 1..index + 3)?;
            if !hex.iter().all(u8::is_ascii_hexdigit) {
                return None;
            }
            decoded.push(u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decoding_reads_gateway_ids_and_rejects_broken_escapes() {
        assert_eq!(
            percent_decode("gateway%3Ahttps%3A%2F%2Fllm.corp%3A443").as_deref(),
            Some("gateway:https://llm.corp:443")
        );
        assert_eq!(percent_decode("oauth:u-1").as_deref(), Some("oauth:u-1"));
        assert_eq!(percent_decode("bad%2"), None);
        assert_eq!(percent_decode("bad%zz"), None);
        assert_eq!(percent_decode("bad%+1"), None);
    }

    #[test]
    fn all_keeps_everything_and_an_account_only_its_sessions() {
        let only = Some(HashSet::from(["s1".to_string()]));
        assert!(Scope::All.keeps(&None, "s9"));
        assert!(Scope::Account("a".into()).keeps(&only, "s1"));
        assert!(!Scope::Account("a".into()).keeps(&only, "s9"));
    }
}
