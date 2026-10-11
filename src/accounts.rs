use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::store::{AccountInfo, AccountKind, AccountRow, AttributionSource, Store, Writer};

#[derive(Debug, Clone, PartialEq)]
pub enum Login {
    SignedIn(AccountRow),
    SignedOut,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccountRef {
    pub id: String,
    pub label: String,
    pub kind: AccountKind,
}

impl From<AccountInfo> for AccountRef {
    fn from(info: AccountInfo) -> Self {
        Self {
            id: info.id,
            label: info.label,
            kind: info.kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Auth {
    pub provider: String,
    pub base_url: Option<String>,
    pub api_key: bool,
}

const ANTHROPIC_API_HOST: &str = "api.anthropic.com";

pub fn read_login(claude_json: &Path) -> Result<Login> {
    let text = std::fs::read_to_string(claude_json)
        .with_context(|| format!("cannot read {}", claude_json.display()))?;
    let value: Value = serde_json::from_str(&text)
        .with_context(|| format!("{} is not valid JSON", claude_json.display()))?;
    let Some(account) = value.get("oauthAccount").filter(|a| a.is_object()) else {
        return Ok(Login::SignedOut);
    };
    let text_of = |key: &str| account.get(key).and_then(Value::as_str).map(String::from);
    let uuid = text_of("accountUuid").context("oauthAccount has no accountUuid")?;
    let organization_type = text_of("organizationType");
    let label = match (text_of("organizationName"), organization_type.as_deref()) {
        (Some(name), Some(kind)) => format!("{name} · {}", plan_name(kind)),
        (Some(name), None) => name,
        (None, Some(kind)) => plan_name(kind),
        (None, None) => "Claude account".to_string(),
    };
    Ok(Login::SignedIn(AccountRow {
        id: format!("oauth:{uuid}"),
        kind: AccountKind::Oauth,
        label,
        organization_type,
        billing_type: text_of("billingType"),
    }))
}

pub fn plan_name(organization_type: &str) -> String {
    match organization_type {
        "claude_max" => "Max",
        "claude_pro" => "Pro",
        "claude_team" => "Team",
        "claude_enterprise" => "Enterprise",
        other => other,
    }
    .to_string()
}

pub fn observe(store: &mut Store, claude_json: &Path, now_ms: i64) -> Result<Option<String>> {
    let login = read_login(claude_json)?;
    let mut writer = store.writer()?;
    let open = writer.open_account()?;
    let now_open = match login {
        Login::SignedIn(row) => {
            let unchanged =
                open.as_deref() == Some(row.id.as_str()) && writer.account_is_current(&row)?;
            if unchanged {
                return Ok(open);
            }
            writer.upsert_account(&row, now_ms)?;
            if open.as_deref() != Some(row.id.as_str()) {
                writer.close_open_period(now_ms)?;
                writer.open_period(&row.id, now_ms)?;
            }
            Some(row.id)
        }
        Login::SignedOut => {
            writer.close_open_period(now_ms)?;
            None
        }
    };
    writer.commit()?;
    Ok(now_open)
}

pub fn auth_account(auth: &Auth) -> Option<AccountRow> {
    let row = |id: String, kind, label: String| AccountRow {
        id,
        kind,
        label,
        organization_type: None,
        billing_type: None,
    };
    if auth.provider != "anthropic" {
        let label = capitalized(&auth.provider);
        return Some(row(
            format!("cloud:{}", auth.provider),
            AccountKind::Cloud,
            label,
        ));
    }
    if let Some((origin, host)) = auth
        .base_url
        .as_deref()
        .and_then(origin_of)
        .filter(|(_, host)| host != ANTHROPIC_API_HOST)
    {
        return Some(row(format!("gateway:{origin}"), AccountKind::Gateway, host));
    }
    auth.api_key
        .then(|| row("api".into(), AccountKind::Api, "API key".into()))
}

fn origin_of(base_url: &str) -> Option<(String, String)> {
    let base_url = base_url.trim();
    let (scheme, rest) = base_url.split_once("://").unwrap_or(("https", base_url));
    let scheme = scheme.to_ascii_lowercase();
    let authority = rest.split(['/', '?', '#']).next()?.to_ascii_lowercase();
    let authority = authority
        .rsplit_once('@')
        .map_or(authority.as_str(), |(_, host)| host);
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if port.bytes().all(|b| b.is_ascii_digit()) && !port.is_empty() => {
            (host.to_string(), port.to_string())
        }
        _ => {
            let port = match scheme.as_str() {
                "https" => "443",
                "http" => "80",
                _ => return None,
            };
            (authority.to_string(), port.to_string())
        }
    };
    (!host.is_empty()).then(|| (format!("{scheme}://{host}:{port}"), host))
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

pub fn attribute_start(
    writer: &mut Writer,
    session: &str,
    auth: Option<&Auth>,
    open: Option<&str>,
    now_ms: i64,
) -> Result<()> {
    let resolved = auth.and_then(auth_account);
    let account = match &resolved {
        Some(row) => {
            writer.upsert_account(row, now_ms)?;
            Some(row.id.as_str())
        }
        None => open,
    };
    if let Some(account) = account {
        writer.attribute(session, account, AttributionSource::Start)?;
    }
    Ok(())
}

pub fn attribute_pending(store: &mut Store) -> Result<usize> {
    let pending = store.unattributed_sessions()?;
    let mut writer = store.writer()?;
    let mut attributed = 0;
    for (session, started_at) in pending {
        if let Some(account) = writer.account_at(started_at.saturating_mul(1000))? {
            attributed +=
                usize::from(writer.attribute(&session, &account, AttributionSource::Timeline)?);
        }
    }
    writer.commit()?;
    Ok(attributed)
}

pub fn account_for_sample(
    writer: &mut Writer,
    session: &str,
    at_ms: i64,
) -> Result<Option<String>> {
    if let Some(account) = writer.session_account(session)? {
        return Ok(Some(account));
    }
    let started_ms = writer
        .session_started_at(session)?
        .map_or(at_ms, |started| started.saturating_mul(1000));
    let Some(account) = writer.account_at(started_ms)? else {
        return Ok(None);
    };
    writer.attribute(session, &account, AttributionSource::Timeline)?;
    Ok(Some(account))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::SessionRow;

    fn login_file(dir: &Path, body: &str) -> std::path::PathBuf {
        let path = dir.join(".claude.json");
        std::fs::write(&path, body).unwrap();
        path
    }

    const ACME: &str = r#"{"oauthAccount":{"accountUuid":"u-1","emailAddress":"x@y","organizationName":"Acme","organizationType":"claude_enterprise","billingType":"invoice"}}"#;
    const MINE: &str = r#"{"oauthAccount":{"accountUuid":"u-2","organizationName":"Me","organizationType":"claude_max","billingType":"stripe_subscription"}}"#;

    #[test]
    fn a_login_is_read_without_the_email() {
        let dir = tempfile::tempdir().unwrap();
        let Login::SignedIn(row) = read_login(&login_file(dir.path(), ACME)).unwrap() else {
            panic!("signed in expected");
        };
        assert_eq!(
            row,
            AccountRow {
                id: "oauth:u-1".into(),
                kind: AccountKind::Oauth,
                label: "Acme · Enterprise".into(),
                organization_type: Some("claude_enterprise".into()),
                billing_type: Some("invoice".into()),
            }
        );
        assert_eq!(
            read_login(&login_file(dir.path(), "{}")).unwrap(),
            Login::SignedOut
        );
    }

    #[test]
    fn plans_have_friendly_names_and_unknown_ones_stay_raw() {
        assert_eq!(plan_name("claude_max"), "Max");
        assert_eq!(plan_name("claude_pro"), "Pro");
        assert_eq!(plan_name("claude_team"), "Team");
        assert_eq!(plan_name("claude_enterprise"), "Enterprise");
        assert_eq!(plan_name("claude_edu"), "claude_edu");
    }

    #[test]
    fn switching_logins_closes_one_period_and_opens_the_next() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_in_memory().unwrap();
        let path = login_file(dir.path(), ACME);
        assert_eq!(
            observe(&mut store, &path, 1_000).unwrap().as_deref(),
            Some("oauth:u-1")
        );
        assert_eq!(
            observe(&mut store, &path, 2_000).unwrap().as_deref(),
            Some("oauth:u-1")
        );
        login_file(dir.path(), MINE);
        assert_eq!(
            observe(&mut store, &path, 3_000).unwrap().as_deref(),
            Some("oauth:u-2")
        );
        login_file(dir.path(), "{}");
        assert_eq!(observe(&mut store, &path, 4_000).unwrap(), None);
        assert_eq!(
            store.account_at(2_500).unwrap().as_deref(),
            Some("oauth:u-1")
        );
        assert_eq!(
            store.account_at(3_500).unwrap().as_deref(),
            Some("oauth:u-2")
        );
        assert_eq!(store.account_at(4_500).unwrap(), None);
    }

    #[test]
    fn a_malformed_or_missing_file_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_in_memory().unwrap();
        let path = login_file(dir.path(), ACME);
        observe(&mut store, &path, 1_000).unwrap();
        login_file(dir.path(), r#"{"oauthAccount":{"accountUu"#);
        assert!(observe(&mut store, &path, 2_000).is_err());
        std::fs::remove_file(&path).unwrap();
        assert!(observe(&mut store, &path, 3_000).is_err());
        assert_eq!(store.open_account().unwrap().as_deref(), Some("oauth:u-1"));
    }

    #[test]
    fn auth_resolves_cloud_then_gateway_then_api_then_nothing() {
        let auth = |provider: &str, base_url: Option<&str>, api_key: bool| Auth {
            provider: provider.into(),
            base_url: base_url.map(String::from),
            api_key,
        };
        let id = |auth: Auth| auth_account(&auth).map(|row| (row.id, row.label));
        assert_eq!(
            id(auth("bedrock", Some("https://gw"), true)),
            Some(("cloud:bedrock".into(), "Bedrock".into()))
        );
        assert_eq!(
            id(auth("anthropic", Some("HTTPS://LLM.Corp.example/v1"), true)),
            Some((
                "gateway:https://llm.corp.example:443".into(),
                "llm.corp.example".into()
            ))
        );
        assert_eq!(
            id(auth("anthropic", Some("http://localhost:4000"), false)),
            Some(("gateway:http://localhost:4000".into(), "localhost".into()))
        );
        assert_eq!(
            id(auth("anthropic", None, true)),
            Some(("api".into(), "API key".into()))
        );
        assert_eq!(id(auth("anthropic", Some(""), false)), None);
    }

    #[test]
    fn pending_sessions_take_the_account_of_their_start_time() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_in_memory().unwrap();
        let path = login_file(dir.path(), ACME);
        observe(&mut store, &path, 10_000).unwrap();
        login_file(dir.path(), MINE);
        observe(&mut store, &path, 20_000).unwrap();
        let mut w = store.writer().unwrap();
        for (id, started_at) in [("before", 5), ("acme", 15), ("mine", 25)] {
            w.session(&SessionRow {
                id: id.into(),
                project: "p".into(),
                cwd: None,
                ts: Some(started_at),
            })
            .unwrap();
        }
        w.commit().unwrap();
        assert_eq!(attribute_pending(&mut store).unwrap(), 2);
        assert_eq!(store.session_account("before").unwrap(), None);
        assert_eq!(
            store.session_account("acme").unwrap().as_deref(),
            Some("oauth:u-1")
        );
        assert_eq!(
            store.session_account("mine").unwrap().as_deref(),
            Some("oauth:u-2")
        );
    }

    #[test]
    fn a_start_without_auth_takes_the_open_login_and_with_auth_its_own_account() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.upsert_account(
            &AccountRow {
                id: "oauth:u-1".into(),
                kind: AccountKind::Oauth,
                label: "A".into(),
                organization_type: None,
                billing_type: None,
            },
            0,
        )
        .unwrap();
        attribute_start(&mut w, "s1", None, Some("oauth:u-1"), 0).unwrap();
        let gateway = Auth {
            provider: "anthropic".into(),
            base_url: Some("https://gw".into()),
            api_key: false,
        };
        attribute_start(&mut w, "s2", Some(&gateway), Some("oauth:u-1"), 0).unwrap();
        attribute_start(&mut w, "s3", None, None, 0).unwrap();
        w.commit().unwrap();
        assert_eq!(
            store.session_account("s1").unwrap().as_deref(),
            Some("oauth:u-1")
        );
        assert_eq!(
            store.session_account("s2").unwrap().as_deref(),
            Some("gateway:https://gw:443")
        );
        assert_eq!(store.session_account("s3").unwrap(), None);
        assert_eq!(
            store
                .account("gateway:https://gw:443")
                .unwrap()
                .unwrap()
                .label,
            "gw"
        );
    }

    #[test]
    fn an_account_ref_carries_id_label_and_kind() {
        let info = AccountInfo {
            id: "api".into(),
            kind: AccountKind::Api,
            label: "API key".into(),
            organization_type: None,
            first_seen_ms: 5,
        };
        assert_eq!(
            AccountRef::from(info),
            AccountRef {
                id: "api".into(),
                label: "API key".into(),
                kind: AccountKind::Api
            }
        );
    }

    fn start_session(store: &mut Store, id: &str, started_at: i64) {
        let mut w = store.writer().unwrap();
        w.session(&SessionRow {
            id: id.into(),
            project: "p".into(),
            cwd: None,
            ts: Some(started_at),
        })
        .unwrap();
        w.commit().unwrap();
    }

    #[test]
    fn a_sample_takes_the_account_of_its_session_start_not_of_the_measure_time() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_in_memory().unwrap();
        let path = login_file(dir.path(), ACME);
        observe(&mut store, &path, 10_000).unwrap();
        login_file(dir.path(), MINE);
        observe(&mut store, &path, 20_000).unwrap();
        start_session(&mut store, "early", 15);
        let mut writer = store.writer().unwrap();
        assert_eq!(
            account_for_sample(&mut writer, "early", 25_000)
                .unwrap()
                .as_deref(),
            Some("oauth:u-1")
        );
        assert_eq!(
            account_for_sample(&mut writer, "unknown", 25_000)
                .unwrap()
                .as_deref(),
            Some("oauth:u-2")
        );
    }

    #[test]
    fn an_observation_that_changes_nothing_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_in_memory().unwrap();
        let path = login_file(dir.path(), ACME);
        observe(&mut store, &path, 1_000).unwrap();
        let before = store.total_changes();
        observe(&mut store, &path, 2_000).unwrap();
        assert_eq!(store.total_changes(), before);
        login_file(
            dir.path(),
            &ACME.replace("claude_enterprise", "claude_team"),
        );
        observe(&mut store, &path, 3_000).unwrap();
        assert!(store.total_changes() > before);
        assert_eq!(
            store.accounts().unwrap()[0].organization_type.as_deref(),
            Some("claude_team")
        );
    }

    #[test]
    fn a_base_url_without_a_scheme_is_still_a_gateway() {
        let gateway = |url: &str| {
            auth_account(&Auth {
                provider: "anthropic".into(),
                base_url: Some(url.into()),
                api_key: false,
            })
        };
        assert_eq!(
            gateway("llm.corp:8080").unwrap().id,
            "gateway:https://llm.corp:8080"
        );
        let bare = gateway("LLM.corp").unwrap();
        assert_eq!(bare.id, "gateway:https://llm.corp:443");
        assert_eq!(bare.label, "llm.corp");
        assert_eq!(gateway("").map(|row| row.kind), None);
        assert_eq!(gateway("   ").map(|row| row.kind), None);
    }

    #[test]
    fn the_anthropic_api_as_a_base_url_is_no_gateway() {
        let auth = |url: &str, api_key: bool| {
            auth_account(&Auth {
                provider: "anthropic".into(),
                base_url: Some(url.into()),
                api_key,
            })
            .map(|row| row.id)
        };
        for url in [
            "https://api.anthropic.com",
            "https://API.Anthropic.com:443/v1/",
            "api.anthropic.com",
            "https://user:secret@api.anthropic.com?x=1",
        ] {
            assert_eq!(auth(url, false), None, "{url}");
            assert_eq!(auth(url, true).as_deref(), Some("api"), "{url}");
        }
        assert_eq!(
            auth("https://api.anthropic.com.corp.example", false).as_deref(),
            Some("gateway:https://api.anthropic.com.corp.example:443")
        );
    }
}
