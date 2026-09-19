use mirador_aiu::{
    backend::{Action, WorkerReply, WorkerRequest},
    model::parse_accounts,
};
use std::{
    fs,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use tempfile::TempDir;
mod support;

fn request(req: WorkerRequest) -> (WorkerReply, Vec<u8>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_mirador-aiu"))
        .arg("--worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    serde_json::to_writer(child.stdin.as_mut().unwrap(), &req).unwrap();
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    (serde_json::from_slice(&out.stdout).unwrap(), out.stderr)
}

fn req(scenario: &str, action: Action) -> (TempDir, WorkerRequest) {
    let dir = TempDir::new().unwrap();
    let request = WorkerRequest {
        aiu_command: vec![
            support::fake_aiu::fake_aiu().to_string_lossy().into(),
            scenario.into(),
        ],
        cwd: dir.path().to_string_lossy().into(),
        provider: Some("claude".into()),
        action,
    };
    (dir, request)
}

#[test]
fn worker_messages_use_stable_tagged_contract() {
    let request = WorkerRequest {
        aiu_command: vec!["aiu".into()],
        cwd: ".".into(),
        provider: Some("claude".into()),
        action: Action::Switch {
            target: "claude:a@example.test".into(),
        },
    };
    let value = serde_json::to_value(&request).unwrap();
    assert_eq!(value["action"]["kind"], "switch");
    assert_eq!(value["action"]["target"], "claude:a@example.test");
    let reply = WorkerReply::Switched {
        target: "claude:a@example.test".into(),
        warnings: false,
    };
    assert_eq!(serde_json::to_value(reply).unwrap()["kind"], "switched");
}

#[test]
fn published_account_parser_rejects_invalid_metadata() {
    let invalid = br#"[{"key":"bad","provider":"claude"}]"#;
    assert!(parse_accounts(invalid).is_err());
}

#[test]
fn worker_runs_fake_status_and_switch_with_warning_bit() {
    let (_d, r) = req("ok", Action::Status);
    let (reply, stderr) = request(r);
    assert!(matches!(reply, WorkerReply::Status { accounts } if accounts.len() == 1));
    assert!(stderr.is_empty());
    let (_d, r) = req(
        "switch_warn",
        Action::Switch {
            target: "claude:a@example.test".into(),
        },
    );
    let (reply, stderr) = request(r);
    assert!(matches!(
        reply,
        WorkerReply::Switched { warnings: true, .. }
    ));
    assert!(stderr.is_empty());
}

#[test]
fn worker_maps_failures_without_exposing_stderr() {
    for scenario in ["fail", "malformed", "oversize"] {
        let (_d, r) = req(scenario, Action::Status);
        let (reply, stderr) = request(r);
        assert!(!String::from_utf8_lossy(&stderr).contains("secret"));
        assert!(matches!(reply, WorkerReply::Error { .. }));
    }
}

#[test]
fn worker_reports_missing_aiu_without_raw_error() {
    let dir = TempDir::new().unwrap();
    let r = WorkerRequest {
        aiu_command: vec![dir.path().join("does-not-exist").to_string_lossy().into()],
        cwd: dir.path().to_string_lossy().into(),
        provider: Some("claude".into()),
        action: Action::Status,
    };
    let (reply, stderr) = request(r);
    assert!(matches!(reply, WorkerReply::Error { code } if code == "missing_aiu"));
    assert!(stderr.is_empty());
}

#[test]
fn worker_rejects_incomplete_account_key_before_execution() {
    let (directory, req) = req(
        "ok",
        Action::Switch {
            target: "claude:".into(),
        },
    );
    let (reply, stderr) = request(req);
    assert!(matches!(reply, WorkerReply::Error { code } if code == "worker_failed"));
    assert!(stderr.is_empty());
    assert!(!directory.path().join("started").exists());
}

#[test]
fn worker_busy_lock_and_natural_completion() {
    let (dir, first_req) = req("gated", Action::Status);
    let mut first = Command::new(env!("CARGO_BIN_EXE_mirador-aiu"))
        .arg("--worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    serde_json::to_writer(first.stdin.as_mut().unwrap(), &first_req).unwrap();
    drop(first.stdin.take());
    let deadline = Instant::now() + Duration::from_secs(3);
    while !dir.path().join("started").exists() {
        assert!(Instant::now() < deadline, "fake AIU did not start");
        thread::sleep(Duration::from_millis(10));
    }
    let (second, _) = request(first_req.clone());
    assert!(matches!(second, WorkerReply::Error { code } if code == "busy"));
    drop(first.stdout.take());
    fs::write(dir.path().join("release"), b"1").unwrap();
    let out = first.wait_with_output().unwrap();
    assert!(out.status.success());
    assert!(dir.path().join("finished").exists());
}
