use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub aiu_command: Vec<String>,
    pub poll_seconds: u64,
    pub slow_after_seconds: u64,
    pub provider: String,
    pub allow_switch: bool,
    pub show_email: bool,
    pub demo: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            aiu_command: vec!["aiu".into()],
            poll_seconds: 60,
            slow_after_seconds: 30,
            provider: "all".into(),
            allow_switch: true,
            show_email: false,
            demo: false,
        }
    }
}

impl Config {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.aiu_command.is_empty()
            || self.aiu_command.len() > 32
            || self.aiu_command[0].trim().is_empty()
            || self
                .aiu_command
                .iter()
                .any(|a| a.len() > 4096 || a.contains('\0'))
        {
            return Err(
                "aiu_command must be a non-empty argv array with at most 32 bounded arguments",
            );
        }
        if !(30..=3600).contains(&self.poll_seconds) {
            return Err("poll_seconds must be between 30 and 3600");
        }
        if !(5..=300).contains(&self.slow_after_seconds) {
            return Err("slow_after_seconds must be between 5 and 300");
        }
        if !["all", "claude", "codex"].contains(&self.provider.as_str()) {
            return Err("provider must be all, claude, or codex");
        }
        Ok(())
    }

    pub fn provider_arg(&self) -> Option<String> {
        (self.provider != "all").then(|| self.provider.clone())
    }
}
