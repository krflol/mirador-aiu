use crate::model::parse_accounts;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
};

const MAX_REQUEST: usize = 160 * 1024;
const MAX_OUTPUT: usize = 2 * 1024 * 1024;
const MAX_ARG: usize = 4096;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    Status,
    Switch { target: String },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WorkerRequest {
    pub aiu_command: Vec<String>,
    pub cwd: String,
    pub provider: Option<String>,
    pub action: Action,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkerReply {
    Status {
        accounts: Vec<crate::model::Account>,
    },
    Switched {
        target: String,
        warnings: bool,
    },
    Error {
        code: String,
    },
}

pub fn spawn_worker(request: WorkerRequest) -> io::Result<mpsc::Receiver<WorkerReply>> {
    let bytes =
        serde_json::to_vec(&request).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    if bytes.len() > MAX_REQUEST {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "worker request too large",
        ));
    }
    let exe = std::env::current_exe()?;
    let mut command = Command::new(exe);
    command
        .arg("--worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    isolate_process(&mut command);
    let mut child = command.spawn()?;
    let mut stdin = child.stdin.take().expect("piped worker stdin");
    stdin.write_all(&bytes)?;
    drop(stdin);
    let stdout = child.stdout.take().expect("piped worker stdout");
    let stderr = child.stderr.take().expect("piped worker stderr");
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let out_thread = thread::spawn(move || read_bounded(stdout));
        let err_thread = thread::spawn(move || discard_stream(stderr));
        let output = out_thread.join().unwrap_or(Err(ReadState::Failed));
        let _stderr_nonempty = err_thread.join().unwrap_or(false);
        let status = child.wait();
        let reply = match (status, output) {
            (Ok(status), Ok(bytes)) if status.success() => decode_reply(&bytes),
            (Ok(_), Ok(_)) => WorkerReply::Error {
                code: "cli_failed".into(),
            },
            (_, Err(ReadState::TooLarge)) => WorkerReply::Error {
                code: "output_too_large".into(),
            },
            _ => WorkerReply::Error {
                code: "worker_failed".into(),
            },
        };
        let _ = tx.send(reply);
    });
    Ok(rx)
}

pub fn run_worker() -> io::Result<()> {
    let mut input = Vec::new();
    io::stdin()
        .take((MAX_REQUEST + 1) as u64)
        .read_to_end(&mut input)?;
    let reply = if input.len() > MAX_REQUEST {
        WorkerReply::Error {
            code: "worker_failed".into(),
        }
    } else {
        match serde_json::from_slice::<WorkerRequest>(&input) {
            Ok(request) => execute(request),
            Err(_) => WorkerReply::Error {
                code: "worker_failed".into(),
            },
        }
    };
    let bytes = serde_json::to_vec(&reply).map_err(io::Error::other)?;
    match io::stdout()
        .write_all(&bytes)
        .and_then(|_| io::stdout().write_all(b"\n"))
    {
        Ok(()) => Ok(()),
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::BrokenPipe | io::ErrorKind::UnexpectedEof
            ) =>
        {
            Ok(())
        }
        Err(e) => Err(e),
    }
}

fn execute(request: WorkerRequest) -> WorkerReply {
    if let Err(code) = validate(&request) {
        return WorkerReply::Error { code: code.into() };
    }
    let _lock = match command_lock(&request) {
        Ok(Some(lock)) => lock,
        Ok(None) => {
            return WorkerReply::Error {
                code: "busy".into(),
            };
        }
        Err(_) => {
            return WorkerReply::Error {
                code: "worker_failed".into(),
            };
        }
    };
    let mut argv = request.aiu_command.clone();
    argv.push("--json".into());
    if let Some(provider) = &request.provider {
        argv.push("--provider".into());
        argv.push(provider.clone());
    }
    match &request.action {
        Action::Status => argv.push("status".into()),
        Action::Switch { target } => {
            argv.push("switch".into());
            argv.push("--".into());
            argv.push(target.clone());
        }
    }
    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .current_dir(&request.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    isolate_process(&mut command);
    let Ok(mut child) = command.spawn() else {
        return WorkerReply::Error {
            code: "missing_aiu".into(),
        };
    };
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let out_thread = thread::spawn(move || read_bounded(stdout));
    let err_thread = thread::spawn(move || discard_stream(stderr));
    let output = out_thread.join().unwrap_or(Err(ReadState::Failed));
    let warnings = err_thread.join().unwrap_or(false);
    let Ok(status) = child.wait() else {
        return WorkerReply::Error {
            code: "worker_failed".into(),
        };
    };
    if !status.success() {
        return WorkerReply::Error {
            code: "cli_failed".into(),
        };
    }
    match output {
        Err(ReadState::TooLarge) => WorkerReply::Error {
            code: "output_too_large".into(),
        },
        Err(_) => WorkerReply::Error {
            code: "worker_failed".into(),
        },
        Ok(bytes) => decode_action_output(&request.action, &bytes, warnings),
    }
}

fn decode_reply(bytes: &[u8]) -> WorkerReply {
    serde_json::from_slice(bytes).unwrap_or(WorkerReply::Error {
        code: "invalid_output".into(),
    })
}

fn decode_action_output(action: &Action, bytes: &[u8], warnings: bool) -> WorkerReply {
    match action {
        Action::Status => parse_accounts(bytes)
            .map(|accounts| WorkerReply::Status { accounts })
            .unwrap_or(WorkerReply::Error {
                code: "invalid_output".into(),
            }),
        Action::Switch { target } => {
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
                return WorkerReply::Error {
                    code: "invalid_output".into(),
                };
            };
            if value.get("switched").and_then(|v| v.as_str()) == Some(target) {
                WorkerReply::Switched {
                    target: target.clone(),
                    warnings,
                }
            } else {
                WorkerReply::Error {
                    code: "invalid_output".into(),
                }
            }
        }
    }
}

#[derive(Debug)]
enum ReadState {
    TooLarge,
    Failed,
}
fn read_bounded(mut reader: impl Read) -> Result<Vec<u8>, ReadState> {
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    let mut oversized = false;
    loop {
        match reader.read(&mut buf) {
            Ok(0) => {
                return if oversized {
                    Err(ReadState::TooLarge)
                } else {
                    Ok(out)
                };
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Ok(n) => {
                if out.len() + n > MAX_OUTPUT {
                    oversized = true;
                } else if !oversized {
                    out.extend_from_slice(&buf[..n]);
                }
            }
            Err(_) => return Err(ReadState::Failed),
        }
    }
}
fn discard_stream(mut reader: impl Read) -> bool {
    let mut buf = [0u8; 8192];
    let mut seen = false;
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Ok(n) => seen |= buf[..n].iter().any(|b| !b.is_ascii_whitespace()),
            Err(_) => break,
        }
    }
    seen
}

fn validate(r: &WorkerRequest) -> Result<(), &'static str> {
    if r.aiu_command.is_empty()
        || r.aiu_command.len() > 32
        || r.aiu_command
            .iter()
            .any(|a| a.is_empty() || a.len() > MAX_ARG || a.contains('\0'))
    {
        return Err("worker_failed");
    }
    if r.cwd.is_empty()
        || r.cwd.len() > MAX_ARG
        || r.cwd.contains('\0')
        || !Path::new(&r.cwd).is_dir()
    {
        return Err("worker_failed");
    }
    if let Some(p) = &r.provider
        && !["claude", "codex"].contains(&p.as_str())
    {
        return Err("worker_failed");
    }
    if let Action::Switch { target } = &r.action
        && (target.is_empty()
            || target.len() > 1024
            || target.chars().any(char::is_control)
            || (!target.starts_with("claude:") && !target.starts_with("codex:"))
            || target
                .split_once(':')
                .is_none_or(|(_, account)| account.is_empty())
            || r.provider
                .as_ref()
                .is_some_and(|provider| !target.starts_with(&format!("{provider}:"))))
    {
        return Err("worker_failed");
    }
    Ok(())
}

struct Lock {
    _file: File,
}
fn command_lock(r: &WorkerRequest) -> io::Result<Option<Lock>> {
    let mut key = r.cwd.clone();
    key.push('|');
    key.push_str(r.provider.as_deref().unwrap_or("all"));
    key.push('|');
    key.push_str(&r.aiu_command.join("\0"));
    let digest = format!("{:x}", Sha256::digest(key.as_bytes()));
    let dir = dirs::cache_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "cache directory unavailable"))?
        .join("mirador-aiu");
    fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    }
    let path = dir.join(format!("{digest}.lock"));
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    match fs2::FileExt::try_lock_exclusive(&file) {
        Ok(()) => Ok(Some(Lock { _file: file })),
        Err(e) if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() => Ok(None),
        Err(e) => Err(e),
    }
}

fn isolate_process(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000 | 0x0000_0200);
    }
}
