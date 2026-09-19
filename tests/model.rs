use mirador_aiu::{config::Config, model::parse_accounts};
use serde_json::json;

#[test]
fn presentation_drops_provider_diagnostics_and_bounds_metadata() {
    let value = json!([{
        "key": "claude:person@example.test",
        "provider": "claude",
        "label": "Safe\u{001b}[31m\nlabel",
        "login": {"state": "ok", "message": "private-provider-detail"},
        "error": "secret-provider-error",
        "stale": "secret-stale-detail",
        "why": "secret-explanation",
        "accessToken": "secret-token-not-in-schema",
        "windows": [{"key": "x".repeat(400), "known": true, "percent": 120, "severity": "locked"}]
    }]);
    let accounts = parse_accounts(&serde_json::to_vec(&value).unwrap()).unwrap();
    let account = &accounts[0];
    let serialized = serde_json::to_string(account).unwrap();
    assert!(!serialized.contains("secret-"));
    assert!(!serialized.contains("private-provider-detail"));
    assert!(!account.label.chars().any(char::is_control));
    assert_eq!(account.windows[0].key.len(), 128);
    assert!(!account.windows[0].known);
    assert_eq!(account.windows[0].severity, "locked");
}

#[test]
fn duplicate_keys_and_legacy_shapes_are_rejected() {
    let account = json!({"key":"codex:person@example.test", "provider":"codex"});
    assert!(parse_accounts(&serde_json::to_vec(&json!([account, account])).unwrap()).is_err());
    assert!(parse_accounts(br#"{"accounts":[]}"#).is_err());
    assert!(parse_accounts(br#"[{"email":"legacy@example.test"}]"#).is_err());
}

#[test]
fn only_confirmed_non_active_writable_logins_are_switchable() {
    for state in [
        "ok",
        "expiring",
        "expired",
        "missing",
        "unknown",
        "unexpected",
    ] {
        for active in [true, false] {
            for read_only in [true, false] {
                let value = json!([{"key":"claude:p@example.test","provider":"claude","active":active,"readOnly":read_only,"login":{"state":state}}]);
                let accounts = parse_accounts(&serde_json::to_vec(&value).unwrap()).unwrap();
                assert_eq!(
                    accounts[0].switchable(),
                    !active && !read_only && ["ok", "expiring"].contains(&state)
                );
            }
        }
    }
}

#[test]
fn configuration_enforces_provider_and_polling_limits() {
    for value in [
        json!({"poll_seconds":29}),
        json!({"poll_seconds":3601}),
        json!({"slow_after_seconds":0}),
        json!({"provider":"other"}),
        json!({"aiu_command":[]}),
    ] {
        let config: Config = serde_json::from_value(value).unwrap();
        assert!(config.validate().is_err());
    }
    assert!(serde_json::from_value::<Config>(json!({"poll_second":60})).is_err());
    assert!(Config::default().validate().is_ok());
}
