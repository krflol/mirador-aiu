use crate::{
    config::Config,
    model::{Account, Window},
    protocol::{Binding, Frame, InputPolicy, Line, Span},
};
use chrono::DateTime;
use unicode_width::UnicodeWidthChar;

const MAX_RECOMMENDATION_AGE_MS: i64 = 15 * 60 * 1000;
const MAX_FUTURE_MS: i64 = 5 * 60 * 1000;

pub struct View<'a> {
    pub accounts: &'a [Account],
    pub selected: usize,
    pub columns: u16,
    pub rows: u16,
    pub busy: bool,
    pub slow: bool,
    pub stale: bool,
    pub notice: &'a str,
    pub confirming: Option<&'a str>,
    pub config: &'a Config,
    pub now_ms: i64,
}

pub fn render(view: &View<'_>, revision: u64) -> Frame {
    let width = view.columns as usize;
    let height = view.rows as usize;
    let selected = view.accounts.get(view.selected);
    let selected_index = view.selected.min(view.accounts.len().saturating_sub(1));
    let mut lines = Vec::new();
    let title = if view.config.demo {
        "AIU · demo"
    } else {
        "AIU · account usage"
    };
    add(&mut lines, height, width, title, "theme:title", true);
    let counter = if view.busy {
        "loading".to_string()
    } else if view.accounts.is_empty() {
        "0/0".into()
    } else {
        format!("{}/{}", selected_index + 1, view.accounts.len())
    };
    add(
        &mut lines,
        height,
        width,
        &format!("{counter}  {}", status_text(view)),
        "theme:muted",
        false,
    );
    if view.config.demo {
        add(
            &mut lines,
            height,
            width,
            "Demo data · account changes are disabled",
            "theme:warning",
            false,
        );
    }
    if !view.notice.is_empty() {
        add(
            &mut lines,
            height,
            width,
            view.notice,
            "theme:warning",
            false,
        );
    }

    // Keep room for the selected account's status and at least two usage rows.
    // The selected row remains visible because the window is centered around it.
    let detail_reserve = selected.map(|a| 3 + a.windows.len().min(2)).unwrap_or(1);
    let available = height.saturating_sub(lines.len());
    let list_budget = available
        .saturating_sub(detail_reserve)
        .max(if available > 0 { 1 } else { 0 })
        .min(view.accounts.len());
    let start = if view.accounts.len() <= list_budget {
        0
    } else {
        selected_index
            .saturating_sub(list_budget / 2)
            .min(view.accounts.len() - list_budget)
    };
    if let Some(target) = view.confirming {
        let label = view
            .accounts
            .iter()
            .enumerate()
            .find(|(_, account)| account.key == target)
            .map(|(index, account)| account_label(account, index, view.config))
            .unwrap_or_else(|| "selected account".into());
        add(
            &mut lines,
            height,
            width,
            &format!("y/n/Esc · {}", short_label(&label)),
            "theme:warning",
            true,
        );
    }
    for (index, account) in view
        .accounts
        .iter()
        .enumerate()
        .skip(start)
        .take(list_budget)
    {
        let mut text = String::new();
        text.push(if index == selected_index { '▶' } else { ' ' });
        text.push(' ');
        text.push_str(&account_label(account, index, view.config));
        let badges = badges(account, recommendation_allowed(account, view));
        if !badges.is_empty() {
            text.push(' ');
            text.push_str(&badges);
        }
        add(
            &mut lines,
            height,
            width,
            &text,
            if index == selected_index {
                "theme:accent"
            } else {
                "theme:text"
            },
            index == selected_index,
        );
    }
    if let Some(account) = selected {
        if lines.len() < height {
            add(&mut lines, height, width, "", "theme:rule", false);
        }
        if lines.len() < height {
            add(
                &mut lines,
                height,
                width,
                &account_label(account, selected_index, view.config),
                "theme:title",
                true,
            );
        }
        if !account.tier.is_empty() && lines.len() < height {
            add(
                &mut lines,
                height,
                width,
                &format!("tier: {}", account.tier),
                "theme:muted",
                false,
            );
        }
        if lines.len() < height {
            add(
                &mut lines,
                height,
                width,
                &account_status(account, view.now_ms),
                "theme:muted",
                false,
            );
        }
        for (window_index, window) in account.windows.iter().enumerate() {
            if lines.len() >= height {
                break;
            }
            let label = if window.label.is_empty() {
                &window.key
            } else {
                &window.label
            };
            let value = if width < 28 {
                usage_bar(window, width.saturating_sub(1).max(1))
            } else {
                let bar_width = width.saturating_sub(25).clamp(5, 24);
                format!("{} {}", short_label(label), usage_bar(window, bar_width))
            };
            add(
                &mut lines,
                height,
                width,
                &value,
                window_color(window),
                false,
            );
            let remaining_windows = account.windows.len().saturating_sub(window_index + 1);
            if !window.resets_at.is_empty()
                && width >= 42
                && lines.len() + remaining_windows < height
            {
                add(
                    &mut lines,
                    height,
                    width,
                    &format!("  resets {}", reset_text(&window.resets_at, view.now_ms)),
                    "theme:muted",
                    false,
                );
            }
        }
        if view.config.show_email && !account.email.is_empty() && lines.len() < height {
            add(
                &mut lines,
                height,
                width,
                &account.email,
                "theme:muted",
                false,
            );
        }
        if !account.org_name.is_empty() && width >= 36 && lines.len() < height {
            add(
                &mut lines,
                height,
                width,
                &account.org_name,
                "theme:muted",
                false,
            );
        }
        if account.read_only && lines.len() < height {
            add(
                &mut lines,
                height,
                width,
                "Read-only login · switching disabled",
                "theme:muted",
                false,
            );
        }
        if !account.switchable() && !account.active && lines.len() < height {
            add(
                &mut lines,
                height,
                width,
                "Account cannot be switched here",
                "theme:muted",
                false,
            );
        }
    } else if lines.len() < height {
        add(
            &mut lines,
            height,
            width,
            "No accounts available",
            "theme:muted",
            false,
        );
    }
    Frame {
        revision,
        title: title.into(),
        counter,
        lines,
        bindings: bindings(view),
        input: InputPolicy {
            capture: false,
            keys: bindings(view).iter().map(|b| b.key.clone()).collect(),
            paste: false,
            mouse: false,
        },
    }
}

fn status_text(view: &View<'_>) -> &'static str {
    if view.busy && view.slow {
        "updating slowly"
    } else if view.busy {
        "updating"
    } else if view.slow {
        "slow response"
    } else if view.stale {
        "cached"
    } else {
        "ready"
    }
}

fn bindings(view: &View<'_>) -> Vec<Binding> {
    if view.confirming.is_some() {
        return [
            ("y", "confirm switch", true),
            ("n", "cancel switch", false),
            ("Esc", "cancel switch", false),
        ]
        .into_iter()
        .map(|(key, action, primary)| Binding {
            key: key.into(),
            action: action.into(),
            primary,
        })
        .collect();
    }
    let mut keys = vec![
        ("Up", "select previous", false),
        ("Down", "select next", false),
        ("j", "select next", false),
        ("k", "select previous", false),
        ("PageUp", "select page up", false),
        ("PageDown", "select page down", false),
        ("Home", "select first", false),
        ("End", "select last", false),
        ("r", "refresh usage", true),
    ];
    if !view.busy
        && !view.config.demo
        && view.config.allow_switch
        && view.columns >= 20
        && view.rows >= 4
        && view
            .accounts
            .get(view.selected)
            .is_some_and(Account::switchable)
    {
        keys.push(("Enter", "switch account", false));
    }
    keys.into_iter()
        .map(|(key, action, primary)| Binding {
            key: key.into(),
            action: action.into(),
            primary,
        })
        .collect()
}

fn account_label(account: &Account, index: usize, config: &Config) -> String {
    let fallback = format!("{} #{}", account.provider, index + 1);
    let label = if account.label.is_empty() {
        fallback
    } else {
        format!("{} · {}", account.provider, account.label)
    };
    if config.show_email && !account.email.is_empty() {
        format!("{label} <{}>", account.email)
    } else {
        label
    }
}

fn badges(account: &Account, recommend: bool) -> String {
    let mut values = Vec::new();
    if account.active {
        values.push("active");
    }
    if recommend {
        values.push("recommended");
    }
    if account.read_only {
        values.push("read-only");
    }
    if account.login.state == "expiring" {
        values.push("expiring");
    }
    values.join(" · ")
}

fn recommendation_allowed(account: &Account, view: &View<'_>) -> bool {
    if !account.recommended || view.stale || !account.error.is_empty() || !account.stale.is_empty()
    {
        return false;
    }
    let age = view.now_ms.saturating_sub(account.fetched_at);
    account.fetched_at > 0
        && (0..=MAX_RECOMMENDATION_AGE_MS).contains(&age)
        && account.fetched_at.saturating_sub(view.now_ms) <= MAX_FUTURE_MS
}

fn account_status(account: &Account, now_ms: i64) -> String {
    let mut parts = Vec::new();
    if account.fetched_at <= 0 {
        parts.push("usage not fetched".into());
    } else {
        let age = now_ms.saturating_sub(account.fetched_at);
        if age < 0 {
            parts.push("usage timestamp ahead".into());
        } else {
            let minutes = age / 60_000;
            parts.push(if minutes < 1 {
                "updated just now".into()
            } else if minutes < 60 {
                format!("updated {minutes}m ago")
            } else {
                format!("updated {}h ago", minutes / 60)
            });
        }
    }
    if !account.stale.is_empty() {
        parts.push("cached usage".into());
    }
    if !account.error.is_empty() {
        parts.push("usage unavailable".into());
    }
    match account.login.state.as_str() {
        "missing" => parts.push("login missing".into()),
        "expired" => parts.push("login expired".into()),
        _ => {}
    }
    parts.join(" · ")
}

fn usage_bar(window: &Window, width: usize) -> String {
    if window.severity == "locked" {
        return format!("[{:^width$}]", "LOCKED", width = width);
    }
    if !window.known {
        return format!("[{:^width$}]", "?", width = width);
    }
    let width = width.max(1);
    let filled = ((window.percent.clamp(0.0, 100.0) / 100.0) * width as f64).round() as usize;
    format!(
        "[{}{}] {:>3.0}%",
        "█".repeat(filled.min(width)),
        "·".repeat(width - filled.min(width)),
        window.percent.clamp(0.0, 100.0)
    )
}

fn window_color(window: &Window) -> &'static str {
    if window.severity == "locked" {
        "theme:error"
    } else if !window.known || window.percent >= 80.0 {
        "theme:warning"
    } else {
        "theme:text"
    }
}

fn short_label(value: &str) -> String {
    value.chars().take(18).collect()
}

fn reset_text(value: &str, now_ms: i64) -> String {
    let Some(parsed) = DateTime::parse_from_rfc3339(value).ok() else {
        return "later".into();
    };
    let delta = parsed.timestamp_millis().saturating_sub(now_ms);
    if delta <= 0 {
        return "ready".into();
    }
    let minutes = (delta + 59_999) / 60_000;
    if minutes < 60 {
        format!("in {minutes}m")
    } else if minutes < 1_440 {
        format!("in {}h", (minutes + 59) / 60)
    } else {
        format!("in {}d", (minutes + 1_439) / 1_440)
    }
}

fn add(lines: &mut Vec<Line>, height: usize, width: usize, text: &str, color: &str, bold: bool) {
    if lines.len() >= height {
        return;
    }
    lines.push(Line {
        spans: vec![Span {
            text: clip(text, width),
            fg: color.into(),
            bold,
        }],
    });
}

fn clip(value: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let mut output = String::new();
    let mut used = 0;
    let mut chars = 0;
    for ch in value.chars() {
        if chars >= 256 || output.len() + ch.len_utf8() > 1024 {
            break;
        }
        if ch.is_control() {
            continue;
        }
        let cell = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + cell > width {
            break;
        }
        output.push(ch);
        chars += 1;
        used += cell;
    }
    output
}
