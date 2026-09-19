use crate::{
    backend::{self, Action, WorkerReply, WorkerRequest},
    config::Config,
    model::{Account, demo_accounts},
    protocol::{self, HostMessage, Outbound},
    render::{self, View},
};
use std::{
    io::{self, BufReader, Write},
    sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError},
    thread,
    time::{Duration, Instant},
};

struct Job {
    receiver: Receiver<WorkerReply>,
    action: Action,
    started: Instant,
}

struct Panel {
    config: Config,
    cwd: String,
    accounts: Vec<Account>,
    selected: usize,
    columns: u16,
    rows: u16,
    notice: String,
    stale: bool,
    confirming: Option<String>,
    job: Option<Job>,
    next_poll: Instant,
    revision: u64,
}

impl Panel {
    fn start(&mut self, action: Action) {
        if self.config.demo || self.job.is_some() {
            return;
        }
        let request = WorkerRequest {
            aiu_command: self.config.aiu_command.clone(),
            cwd: self.cwd.clone(),
            provider: self.config.provider_arg(),
            action: action.clone(),
        };
        let (sender, receiver) = mpsc::sync_channel(1);
        // Process startup and pipe writes also stay off the protocol thread.
        thread::spawn(move || {
            let reply = backend::spawn_worker(request)
                .ok()
                .and_then(|receiver| receiver.recv().ok())
                .unwrap_or_else(|| WorkerReply::Error {
                    code: "worker_failed".into(),
                });
            let _ = sender.send(reply);
        });
        self.job = Some(Job {
            receiver,
            action,
            started: Instant::now(),
        });
        self.confirming = None;
    }

    fn key(&mut self, key: &str) {
        if let Some(target) = self.confirming.clone() {
            match key {
                "y" => {
                    self.confirming = None;
                    let eligible = self
                        .accounts
                        .iter()
                        .any(|a| a.key == target && a.switchable());
                    if eligible
                        && self.config.allow_switch
                        && !self.config.demo
                        && self.job.is_none()
                    {
                        self.notice = "Switching the selected CLI login...".into();
                        self.start(Action::Switch { target });
                    } else {
                        self.notice =
                            "Account state changed. Select an available account again.".into();
                    }
                }
                "n" | "Esc" => {
                    self.confirming = None;
                    self.notice = "Switch cancelled.".into();
                }
                _ => {}
            }
            return;
        }
        let last = self.accounts.len().saturating_sub(1);
        let page = usize::from(self.rows.saturating_sub(6) / 3).max(1);
        match key {
            "Up" | "k" => self.selected = self.selected.saturating_sub(1),
            "Down" | "j" => self.selected = self.selected.saturating_add(1).min(last),
            "PageUp" => self.selected = self.selected.saturating_sub(page),
            "PageDown" => self.selected = self.selected.saturating_add(page).min(last),
            "Home" => self.selected = 0,
            "End" => self.selected = last,
            "r" => {
                if self.config.demo {
                    self.accounts = demo_accounts();
                    self.accounts.retain(|a| {
                        self.config.provider == "all" || a.provider == self.config.provider
                    });
                    self.notice = "Demo refreshed; no AIU command was run.".into();
                } else if self.job.is_none() {
                    self.notice.clear();
                    self.start(Action::Status);
                }
            }
            "Enter"
                if self.config.allow_switch
                    && !self.config.demo
                    && self.job.is_none()
                    && self.columns >= 20
                    && self.rows >= 4 =>
            {
                if let Some(account) = self.accounts.get(self.selected).filter(|a| a.switchable()) {
                    self.confirming = Some(account.key.clone());
                }
            }
            _ => {}
        }
    }

    fn completion(&mut self, writer: &mut impl Write) -> io::Result<bool> {
        let reply = match self.job.as_ref().map(|job| job.receiver.try_recv()) {
            Some(Ok(reply)) => reply,
            Some(Err(TryRecvError::Disconnected)) => WorkerReply::Error {
                code: "worker_failed".into(),
            },
            _ => return Ok(false),
        };
        let job = self.job.take().expect("completion has a running job");
        self.next_poll = Instant::now() + Duration::from_secs(self.config.poll_seconds);
        match (job.action, reply) {
            (Action::Status, WorkerReply::Status { accounts }) => {
                let selected_key = self.accounts.get(self.selected).map(|a| a.key.clone());
                self.accounts = accounts;
                self.accounts.retain(|a| {
                    self.config.provider == "all" || a.provider == self.config.provider
                });
                self.selected = selected_key
                    .and_then(|key| self.accounts.iter().position(|a| a.key == key))
                    .unwrap_or(self.selected.min(self.accounts.len().saturating_sub(1)));
                if self.stale {
                    self.notice.clear();
                }
                self.stale = false;
            }
            (Action::Switch { target: requested }, WorkerReply::Switched { target, warnings })
                if target == requested =>
            {
                if let Some(provider) = self
                    .accounts
                    .iter()
                    .find(|a| a.key == target)
                    .map(|a| a.provider.clone())
                {
                    for account in &mut self.accounts {
                        if account.provider == provider {
                            account.active = account.key == target;
                        }
                    }
                }
                self.notice = if warnings {
                    "Switched with warnings. Check AIU, then start a new CLI session."
                } else {
                    "Switched. Start a new CLI session to use this account."
                }
                .into();
                self.stale = false;
                protocol::write_message(
                    writer,
                    &Outbound::Watch {
                        text: "AIU account switched. Start a new CLI session.",
                    },
                )?;
                self.start(Action::Status);
            }
            (_, WorkerReply::Error { code }) => {
                self.stale = true;
                self.notice = error_notice(&code).into();
            }
            _ => {
                self.stale = true;
                self.notice = error_notice("invalid_output").into();
            }
        }
        Ok(true)
    }

    fn publish(&mut self, writer: &mut impl Write) -> io::Result<()> {
        self.revision = self.revision.saturating_add(1);
        let frame = render::render(
            &View {
                accounts: &self.accounts,
                selected: self.selected,
                columns: self.columns,
                rows: self.rows,
                busy: self.job.is_some(),
                slow: self.job.as_ref().is_some_and(|job| {
                    job.started.elapsed() >= Duration::from_secs(self.config.slow_after_seconds)
                }),
                stale: self.stale,
                notice: &self.notice,
                confirming: self.confirming.as_deref(),
                config: &self.config,
                now_ms: chrono::Utc::now().timestamp_millis(),
            },
            self.revision,
        );
        protocol::write_message(writer, &Outbound::Frame { frame: &frame })
    }
}

fn error_notice(code: &str) -> &'static str {
    match code {
        "missing_aiu" => "AIU was not found. Install aiu-rs or set aiu_command.",
        "cli_failed" => "AIU could not complete the request. Check aiu status for details.",
        "invalid_output" => "AIU returned incompatible data. This panel needs aiu-rs 0.2 or newer.",
        "output_too_large" => "AIU returned too much data. The previous snapshot is retained.",
        "busy" => "Another AIU request is finishing. Refresh again shortly.",
        _ => "The AIU worker could not finish. Check aiu_command and refresh.",
    }
}

enum Input {
    Message(HostMessage),
    Closed,
    Failed,
}

fn fatal(writer: &mut impl Write, message: &'static str) -> io::Result<()> {
    protocol::write_message(
        writer,
        &Outbound::Error {
            message,
            fatal: true,
        },
    )?;
    Err(io::Error::new(io::ErrorKind::InvalidData, message))
}

pub fn run(demo: bool) -> io::Result<()> {
    let mut reader = BufReader::new(io::stdin());
    let mut writer = io::BufWriter::new(io::stdout().lock());
    let first = match protocol::read_message(&mut reader) {
        Ok(Some(message)) => message,
        Ok(None) => return Ok(()),
        Err(_) => return fatal(&mut writer, "Invalid initial protocol message"),
    };
    let HostMessage::Hello {
        protocol,
        config,
        cwd,
        ..
    } = first
    else {
        return fatal(&mut writer, "Expected a Mirador hello message");
    };
    if protocol != protocol::PROTOCOL_VERSION {
        return fatal(
            &mut writer,
            "This plugin requires Mirador external-panel protocol 1",
        );
    }
    let mut config: Config = match serde_json::from_value(config) {
        Ok(config) => config,
        Err(_) => {
            return fatal(
                &mut writer,
                "Invalid plugin configuration; see the mirador-aiu README",
            );
        }
    };
    if let Err(message) = config.validate() {
        return fatal(&mut writer, message);
    }
    config.demo |= demo;
    if config.demo {
        config.allow_switch = false;
    }
    let mut accounts = if config.demo {
        demo_accounts()
    } else {
        Vec::new()
    };
    accounts.retain(|a| config.provider == "all" || a.provider == config.provider);
    let mut panel = Panel {
        config,
        cwd,
        accounts,
        selected: 0,
        columns: 80,
        rows: 24,
        notice: String::new(),
        stale: false,
        confirming: None,
        job: None,
        next_poll: Instant::now(),
        revision: 0,
    };
    protocol::write_message(
        &mut writer,
        &Outbound::Ready {
            protocol: protocol::PROTOCOL_VERSION,
            title: "AIU",
            refresh_ms: 250,
        },
    )?;
    panel.start(Action::Status);
    panel.publish(&mut writer)?;

    let (sender, receiver) = mpsc::sync_channel(8);
    thread::spawn(move || {
        loop {
            let input = match protocol::read_message(&mut reader) {
                Ok(Some(message)) => Input::Message(message),
                Ok(None) => Input::Closed,
                Err(_) => Input::Failed,
            };
            let done = matches!(input, Input::Closed | Input::Failed);
            if sender.send(input).is_err() || done {
                break;
            }
        }
    });
    let mut last_frame = Instant::now();
    let mut dirty = true;
    loop {
        dirty |= panel.completion(&mut writer)?;
        if panel.job.is_none()
            && panel.confirming.is_none()
            && !panel.config.demo
            && Instant::now() >= panel.next_poll
        {
            panel.start(Action::Status);
            dirty = true;
        }
        match receiver.recv_timeout(Duration::from_millis(20)) {
            Ok(Input::Message(HostMessage::Shutdown))
            | Ok(Input::Closed)
            | Err(RecvTimeoutError::Disconnected) => {
                // Existing workers own/reap their AIU child and finish saving credentials.
                // No joins or termination requests can interrupt that transaction here.
                return Ok(());
            }
            Ok(Input::Failed) => return fatal(&mut writer, "Invalid host protocol message"),
            Ok(Input::Message(HostMessage::Hello { .. })) => {
                return fatal(&mut writer, "Protocol hello may only be sent once");
            }
            Ok(Input::Message(HostMessage::Resize { columns, rows })) => {
                panel.columns = columns;
                panel.rows = rows;
                if (columns < 20 || rows < 4) && panel.confirming.take().is_some() {
                    panel.notice =
                        "Switch cancelled. Enlarge the panel to confirm a switch.".into();
                }
                dirty = true;
            }
            Ok(Input::Message(HostMessage::Key { key, .. })) => {
                panel.key(&key);
                dirty = true; // Even an ignored/racing key must acknowledge the host barrier.
            }
            Ok(Input::Message(HostMessage::Focus { .. })) => dirty = true,
            Ok(Input::Message(_)) | Err(RecvTimeoutError::Timeout) => {}
        }
        if (dirty && last_frame.elapsed() >= Duration::from_millis(25))
            || last_frame.elapsed() >= Duration::from_secs(1)
        {
            panel.publish(&mut writer)?;
            last_frame = Instant::now();
            dirty = false;
        }
    }
}
