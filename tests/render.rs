use mirador_aiu::{
    config::Config,
    model::{Account, Login, Window},
    render::{View, render},
};

fn account(key: &str, label: &str, weekly: f64) -> Account {
    Account {
        key: key.into(),
        provider: key.split(':').next().unwrap_or("claude").into(),
        label: label.into(),
        fetched_at: 1_700_000_000_000,
        login: Login { state: "ok".into() },
        windows: vec![Window {
            key: "weekly_all".into(),
            group: "weekly".into(),
            label: "7d all".into(),
            percent: weekly,
            known: true,
            resets_at: "2026-01-02T00:00:00Z".into(),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn view<'a>(accounts: &'a [Account], config: &'a Config) -> View<'a> {
    View {
        accounts,
        selected: 0,
        columns: 80,
        rows: 24,
        busy: false,
        slow: false,
        stale: false,
        notice: "",
        confirming: None,
        config,
        now_ms: 1_700_000_000_000,
    }
}

#[test]
fn renders_locked_unknown_and_bounded_unicode() {
    let mut a = account("claude:a", "A\u{0007}界", 20.0);
    a.windows.push(Window {
        key: "five_hour".into(),
        group: "session".into(),
        label: "Session".into(),
        severity: "locked".into(),
        known: false,
        ..Default::default()
    });
    let cfg = Config {
        show_email: true,
        ..Config::default()
    };
    let accounts = vec![a];
    let frame = render(&view(&accounts, &cfg), 1);
    let text: String = frame
        .lines
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.text.clone())
        .collect();
    assert!(!text.contains('\u{0007}'));
    assert!(text.contains("LOCKED"));
    assert!(frame.lines.iter().all(|l| {
        l.spans
            .iter()
            .all(|s| unicode_width::UnicodeWidthStr::width(s.text.as_str()) <= 80)
    }));
}

#[test]
fn recommendation_badge_requires_fresh_non_stale_evidence() {
    let mut a = account("claude:a", "A", 20.0);
    a.recommended = true;
    let cfg = Config::default();
    let accounts = vec![a];
    let mut v = view(&accounts, &cfg);
    assert!(
        render(&v, 1)
            .lines
            .iter()
            .any(|l| l.spans[0].text.contains("recommended"))
    );
    v.stale = true;
    assert!(
        !render(&v, 2)
            .lines
            .iter()
            .any(|l| l.spans[0].text.contains("recommended"))
    );
}

#[test]
fn confirmation_replaces_normal_keys_and_narrow_view_stays_bounded() {
    let accounts = vec![account("claude:a", "A", 20.0)];
    let cfg = Config::default();
    let mut v = view(&accounts, &cfg);
    v.columns = 12;
    v.rows = 8;
    v.confirming = Some("claude:a");
    let frame = render(&v, 3);
    let keys: Vec<_> = frame.bindings.iter().map(|b| b.key.as_str()).collect();
    assert_eq!(keys, vec!["y", "n", "Esc"]);
    assert!(frame.lines.len() <= 8);
    assert!(
        frame
            .lines
            .iter()
            .any(|l| l.spans[0].text.contains("Claude") || l.spans[0].text.contains("claude"))
    );
    assert!(
        !keys
            .iter()
            .any(|key| ["q", "Tab", "?", "w", "m", "t", "Ctrl+C"].contains(key))
    );
}

#[test]
fn provider_tier_status_and_default_email_are_safe() {
    let mut a = account("claude:a", "Personal", 20.0);
    a.email = "secret@example.com".into();
    a.tier = "pro".into();
    a.stale = "cached".into();
    a.login.state = "expired".into();
    let cfg = Config::default();
    let frame = render(&view(&[a], &cfg), 5);
    let text: String = frame
        .lines
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.text.as_str())
        .collect();
    assert!(text.contains("claude") && text.contains("pro"));
    assert!(text.contains("cached") && text.contains("expired"));
    assert!(!text.contains("secret@example.com"));
}

#[test]
fn selected_account_and_confirmation_survive_small_or_busy_panes() {
    let mut a = account("claude:a", "A", 20.0);
    for i in 0..12 {
        a.windows.push(Window {
            key: format!("w{i}"),
            label: format!("window {i}"),
            ..Default::default()
        });
    }
    let accounts = vec![a];
    let cfg = Config::default();
    let mut v = view(&accounts, &cfg);
    v.columns = 40;
    v.rows = 5;
    v.confirming = Some("claude:a");
    v.busy = true;
    v.slow = true;
    let frame = render(&v, 6);
    assert!(
        frame
            .lines
            .iter()
            .any(|l| l.spans[0].text.contains("y/n/Esc") && l.spans[0].text.contains("A"))
    );
    assert!(
        frame
            .lines
            .iter()
            .any(|l| l.spans[0].text.contains("updating slowly"))
    );
    assert!(
        frame
            .lines
            .iter()
            .any(|l| l.spans[0].text.contains("Personal")
                || l.spans[0].text.contains("Claude")
                || l.spans[0].text.contains("claude"))
    );
}

#[test]
fn tiny_pane_has_no_phantom_lines_and_enter_needs_room() {
    let accounts = vec![account("claude:a", "A", 20.0)];
    let cfg = Config::default();
    let mut v = view(&accounts, &cfg);
    v.columns = 0;
    v.rows = 0;
    assert!(render(&v, 7).lines.is_empty());
    v.columns = 19;
    v.rows = 3;
    assert!(!render(&v, 8).bindings.iter().any(|b| b.key == "Enter"));
}

#[test]
fn compact_selected_details_keep_both_usage_states() {
    let mut a = account("claude:a", "A", 30.0);
    a.windows.push(Window {
        key: "session".into(),
        group: "session".into(),
        label: "session".into(),
        percent: 10.0,
        known: true,
        ..Default::default()
    });
    let accounts = vec![a];
    let cfg = Config::default();
    let mut v = view(&accounts, &cfg);
    v.rows = 12;
    let frame = render(&v, 9);
    let text: String = frame
        .lines
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.text.as_str())
        .collect();
    assert!(text.contains("30%"));
    assert!(text.contains("10%"));
}

#[test]
fn narrow_usage_line_keeps_locked_state_visible() {
    let mut a = account("claude:a", "A very long account label", 20.0);
    a.windows[0].severity = "locked".into();
    a.windows[0].known = false;
    let cfg = Config::default();
    let accounts = vec![a];
    let mut v = view(&accounts, &cfg);
    v.columns = 20;
    let frame = render(&v, 10);
    assert!(
        frame
            .lines
            .iter()
            .any(|l| l.spans[0].text.contains("LOCKED"))
    );
}

#[test]
fn enter_is_not_claimed_when_switch_is_unavailable() {
    let cfg = Config {
        demo: true,
        ..Config::default()
    };
    let accounts = vec![account("claude:a", "A", 20.0)];
    let frame = render(&view(&accounts, &cfg), 4);
    assert!(!frame.bindings.iter().any(|b| b.key == "Enter"));
}
