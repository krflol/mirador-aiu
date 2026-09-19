use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Account {
    pub key: String,
    pub provider: String,
    pub email: String,
    pub label: String,
    pub org_name: String,
    pub tier: String,
    pub active: bool,
    pub read_only: bool,
    pub windows: Vec<Window>,
    pub fetched_at: i64,
    pub stale: String,
    pub error: String,
    pub login: Login,
    pub recommended: bool,
    pub why: String,
    pub all_spent: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Window {
    pub key: String,
    pub group: String,
    pub label: String,
    pub percent: f64,
    pub known: bool,
    pub resets_at: String,
    pub severity: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Login {
    pub state: String,
    // AIU's raw diagnostics are intentionally not retained or rendered.
}

impl Account {
    pub fn switchable(&self) -> bool {
        !self.active
            && !self.read_only
            && ["ok", "expiring"].contains(&self.login.state.as_str())
            && self.valid_key()
    }

    pub fn valid_key(&self) -> bool {
        ["claude", "codex"].contains(&self.provider.as_str())
            && self.key.starts_with(&format!("{}:", self.provider))
            && self.key.len() > self.provider.len() + 1
            && self.key.len() <= 1024
            && !self.key.chars().any(char::is_control)
    }
}

/// Decode the published AIU 0.2 account array, rejecting incompatible data.
pub fn parse_accounts(bytes: &[u8]) -> Result<Vec<Account>, &'static str> {
    let mut accounts: Vec<Account> =
        serde_json::from_slice(bytes).map_err(|_| "AIU returned an incompatible account array")?;
    if accounts.len() > 256 {
        return Err("AIU returned more than 256 accounts");
    }
    let mut keys = std::collections::HashSet::new();
    for account in &mut accounts {
        if !account.valid_key() || !keys.insert(account.key.clone()) || account.windows.len() > 64 {
            return Err("AIU returned invalid or duplicate account metadata");
        }
        // Keep state evidence while dropping free-form provider diagnostic text.
        if !account.error.is_empty() {
            account.error = "Usage unavailable; inspect AIU for details".into();
        }
        if !account.stale.is_empty() {
            account.stale = "Cached usage".into();
        }
        account.why.clear();
        if !["ok", "expiring", "expired", "missing", "unknown"]
            .contains(&account.login.state.as_str())
        {
            account.login.state = "unknown".into();
        }
        for text in [
            &mut account.email,
            &mut account.label,
            &mut account.org_name,
            &mut account.tier,
        ] {
            *text = text.chars().filter(|c| !c.is_control()).take(256).collect();
        }
        for window in &mut account.windows {
            for text in [
                &mut window.key,
                &mut window.group,
                &mut window.resets_at,
                &mut window.severity,
            ] {
                *text = text.chars().filter(|c| !c.is_control()).take(128).collect();
            }
            window.label = window
                .label
                .chars()
                .filter(|c| !c.is_control())
                .take(128)
                .collect();
            if !window.percent.is_finite() || !(0.0..=100.0).contains(&window.percent) {
                window.known = false;
                window.percent = 0.0;
            }
        }
    }
    Ok(accounts)
}

pub fn demo_accounts() -> Vec<Account> {
    let now = chrono::Utc::now();
    let mut accounts = parse_accounts(include_bytes!("../examples/demo-accounts.json"))
        .expect("built-in demo is valid");
    for account in &mut accounts {
        account.fetched_at = now.timestamp_millis();
        for window in &mut account.windows {
            let hours = if window.group == "session" { 3 } else { 72 };
            window.resets_at = (now + chrono::Duration::hours(hours)).to_rfc3339();
        }
    }
    accounts
}
