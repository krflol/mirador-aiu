use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

mod support;
use support::fake_aiu::fake_aiu;

struct Plugin {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
}

impl Drop for Plugin {
    fn drop(&mut self) {
        let _ = self.stdin.take();
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn binary() -> std::path::PathBuf {
    std::env::var_os("CARGO_BIN_EXE_mirador-aiu")
        .or_else(|| std::env::var_os("CARGO_BIN_EXE_mirador_aiu"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let mut path = std::env::current_exe().unwrap();
            path.pop();
            path.pop();
            path.push(if cfg!(windows) {
                "mirador-aiu.exe"
            } else {
                "mirador-aiu"
            });
            path
        })
}

fn plugin(args: &[&str]) -> Plugin {
    let mut child = Command::new(binary())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("mirador-aiu binary must be built for protocol runtime tests");
    let stdout = child.stdout.take().unwrap();
    let (tx, lines) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    Plugin {
        stdin: child.stdin.take(),
        child,
        lines,
    }
}

fn send(plugin: &mut Plugin, message: Value) {
    let stdin = plugin.stdin.as_mut().unwrap();
    serde_json::to_writer(&mut *stdin, &message).unwrap();
    stdin.write_all(b"\n").unwrap();
    stdin.flush().unwrap();
}

fn next(plugin: &Plugin, timeout: Duration) -> Value {
    serde_json::from_str(&plugin.lines.recv_timeout(timeout).expect("protocol output")).unwrap()
}

fn hello() -> Value {
    hello_config(json!({}), ".")
}

fn hello_config(config: Value, cwd: &str) -> Value {
    json!({"type":"hello","protocol":1,"host_version":"test","plugin":"aiu","config":config,"cwd":cwd})
}

fn wait_for_file(path: &std::path::Path, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn frame_with_accounts(plugin: &Plugin) -> Value {
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for account frame"
        );
        let frame = next(plugin, deadline.saturating_duration_since(Instant::now()));
        if frame["type"] == "frame"
            && frame.to_string().contains("Active")
            && frame.to_string().contains("Eligible")
        {
            return frame;
        }
    }
}

fn newer_frame(plugin: &Plugin, revision: u64, timeout: Duration) -> Value {
    let deadline = Instant::now() + timeout;
    loop {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for frame revision > {revision}"
        );
        let frame = next(plugin, deadline.saturating_duration_since(Instant::now()));
        if frame["type"] == "frame" && frame["revision"].as_u64().unwrap_or(0) > revision {
            return frame;
        }
    }
}

fn shutdown(plugin: &mut Plugin) {
    send(plugin, json!({"type":"shutdown"}));
    plugin.stdin.take();
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        if let Some(status) = plugin.child.try_wait().unwrap() {
            assert!(status.success(), "worker exited unsuccessfully: {status}");
            return;
        }
        assert!(Instant::now() < deadline, "shutdown exceeded one second");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn demo_handshake_ready_frame_and_lifecycle_messages() {
    let mut plugin = plugin(&["--demo"]);
    send(&mut plugin, hello());
    let ready = next(&plugin, Duration::from_secs(2));
    assert_eq!(ready["type"], "ready");
    assert_eq!(ready["protocol"], 1);
    let frame = next(&plugin, Duration::from_secs(2));
    assert_eq!(frame["type"], "frame");
    let initial_revision = frame["revision"].as_u64().unwrap();
    assert!(frame["lines"].is_array());
    send(
        &mut plugin,
        json!({"type":"resize","columns":120,"rows":40}),
    );
    send(&mut plugin, json!({"type":"focus","focused":true}));
    send(&mut plugin, json!({"type":"tick"}));
    let updated = next(&plugin, Duration::from_secs(2));
    assert_eq!(updated["type"], "frame");
    assert!(updated["revision"].as_u64().unwrap() > initial_revision);
    for key in updated.as_object().unwrap().keys() {
        assert!(
            matches!(
                key.as_str(),
                "type" | "revision" | "title" | "counter" | "lines" | "bindings" | "input"
            ),
            "reserved frame key leaked: {key}"
        );
    }
    shutdown(&mut plugin);
}

#[test]
fn demo_has_three_accounts_and_never_requires_aiu_execution() {
    let temp = tempfile::tempdir().unwrap();
    let fake = fake_aiu();
    let mut plugin = plugin(&[]);
    let mut demo_hello = hello();
    demo_hello["config"] = json!({
        "demo": true,
        "aiu_command": [fake, "slow"]
    });
    demo_hello["cwd"] = temp.path().to_string_lossy().into();
    send(&mut plugin, demo_hello);
    assert_eq!(next(&plugin, Duration::from_secs(2))["type"], "ready");
    let frame = next(&plugin, Duration::from_secs(2));
    let text = frame.to_string();
    assert!(text.contains("frame"));
    assert!(frame["lines"].as_array().unwrap().len() >= 3);
    send(
        &mut plugin,
        json!({"type":"key","key":"r","code":"char","text":"r","modifiers":[]}),
    );
    assert_eq!(next(&plugin, Duration::from_secs(1))["type"], "frame");
    send(
        &mut plugin,
        json!({"type":"key","key":"Enter","code":"named","text":null,"modifiers":[]}),
    );
    send(
        &mut plugin,
        json!({"type":"key","key":"y","code":"char","text":"y","modifiers":[]}),
    );
    thread::sleep(Duration::from_millis(100));
    assert!(!temp.path().join("started").exists());
    shutdown(&mut plugin);
}

#[test]
fn gated_worker_keeps_key_ack_fast_and_finishes_after_shutdown() {
    let temp = tempfile::tempdir().unwrap();
    let fake = fake_aiu();
    let config = json!({"aiu_command":[fake,"slow"],"allow_switch":false});
    let mut plugin = plugin(&[]);
    send(
        &mut plugin,
        hello_config(config, &temp.path().to_string_lossy()),
    );
    assert_eq!(next(&plugin, Duration::from_secs(2))["type"], "ready");
    let mut baseline = next(&plugin, Duration::from_secs(2))["revision"]
        .as_u64()
        .unwrap();
    wait_for_file(&temp.path().join("started"), Duration::from_secs(2));
    for _ in 0..8 {
        send(
            &mut plugin,
            json!({"type":"key","key":"r","code":"char","text":"r","modifiers":[]}),
        );
        baseline = newer_frame(&plugin, baseline, Duration::from_millis(750))["revision"]
            .as_u64()
            .unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(temp.path().join("started.log"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    send(&mut plugin, json!({"type":"shutdown"}));
    plugin.stdin.take();
    let deadline = Instant::now() + Duration::from_secs(1);
    while plugin.child.try_wait().unwrap().is_none() {
        assert!(
            Instant::now() < deadline,
            "shutdown did not finish promptly"
        );
        thread::sleep(Duration::from_millis(10));
    }
    std::fs::write(temp.path().join("release"), b"ok").unwrap();
    wait_for_file(&temp.path().join("finished"), Duration::from_secs(2));
}

#[test]
fn stateful_switch_requires_confirmation_and_switches_once() {
    let temp = tempfile::tempdir().unwrap();
    let fake = fake_aiu();
    let config = json!({"aiu_command":[fake,"stateful"],"allow_switch":true});
    let mut plugin = plugin(&[]);
    send(
        &mut plugin,
        hello_config(config, &temp.path().to_string_lossy()),
    );
    assert_eq!(next(&plugin, Duration::from_secs(2))["type"], "ready");
    assert_eq!(next(&plugin, Duration::from_secs(2))["type"], "frame");
    let initial = frame_with_accounts(&plugin);
    assert!(initial.to_string().contains("Eligible"));
    let mut baseline = initial["revision"].as_u64().unwrap();
    send(
        &mut plugin,
        json!({"type":"key","key":"End","code":"named","text":null,"modifiers":[]}),
    );
    let selected = newer_frame(&plugin, baseline, Duration::from_secs(1));
    baseline = selected["revision"].as_u64().unwrap();
    assert_eq!(selected["counter"], "2/2");
    send(
        &mut plugin,
        json!({"type":"key","key":"Enter","code":"named","text":null,"modifiers":[]}),
    );
    let confirm = newer_frame(&plugin, baseline, Duration::from_secs(1));
    assert!(confirm.to_string().contains("confirm"));
    baseline = confirm["revision"].as_u64().unwrap();
    send(
        &mut plugin,
        json!({"type":"key","key":"n","code":"char","text":"n","modifiers":[]}),
    );
    let cancelled = newer_frame(&plugin, baseline, Duration::from_secs(1));
    assert!(!cancelled.to_string().contains("confirm"));
    assert!(!temp.path().join("switch").exists());
    send(
        &mut plugin,
        json!({"type":"key","key":"Enter","code":"named","text":null,"modifiers":[]}),
    );
    let confirm_again = newer_frame(
        &plugin,
        cancelled["revision"].as_u64().unwrap(),
        Duration::from_secs(1),
    );
    assert!(confirm_again.to_string().contains("confirm"));
    send(
        &mut plugin,
        json!({"type":"key","key":"y","code":"char","text":"y","modifiers":[]}),
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut saw_watch = false;
    let mut saw_second_active = false;
    while Instant::now() < deadline {
        let value = next(&plugin, deadline.saturating_duration_since(Instant::now()));
        saw_watch |= value["type"] == "watch" && value.to_string().contains("switched");
        saw_second_active |= value["type"] == "frame"
            && value["lines"].as_array().is_some_and(|lines| {
                lines.iter().any(|line| {
                    let text = line["spans"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter_map(|span| span["text"].as_str())
                        .collect::<String>();
                    text.contains("Eligible") && text.contains("active")
                })
            });
        if saw_watch && saw_second_active && temp.path().join("switch").exists() {
            break;
        }
    }
    assert!(saw_watch);
    assert!(saw_second_active);
    assert_eq!(
        std::fs::read_to_string(temp.path().join("switch-target")).unwrap(),
        "claude:eligible@example.test"
    );
    assert_eq!(
        std::fs::read_to_string(temp.path().join("switch"))
            .unwrap()
            .matches("switch")
            .count(),
        1
    );
    shutdown(&mut plugin);
}

#[test]
fn interactive_keys_receive_immediate_frame_acknowledgements() {
    let mut plugin = plugin(&["--demo"]);
    send(&mut plugin, hello());
    assert_eq!(next(&plugin, Duration::from_secs(2))["type"], "ready");
    assert_eq!(next(&plugin, Duration::from_secs(2))["type"], "frame");
    for key in ["j", "k", "PageDown", "Home", "End", "Enter", "Esc"] {
        send(
            &mut plugin,
            json!({"type":"key","key":key,"code":"named","text":null,"modifiers":[]}),
        );
        assert_eq!(next(&plugin, Duration::from_secs(1))["type"], "frame");
    }
    shutdown(&mut plugin);
}

#[test]
fn version_mismatch_and_unknown_fields_are_generic_fatal_errors() {
    let cases = [
        json!({"type":"hello","protocol":99,"host_version":"test","plugin":"aiu","config":{},"cwd":"."}),
        json!({"type":"hello","protocol":1,"host_version":"test","plugin":"aiu","config":{},"cwd":".","sentinel":"private-input"}),
    ];
    for message in cases {
        let raw = message.to_string();
        let mut plugin = plugin(&[]);
        send(&mut plugin, message);
        let error = next(&plugin, Duration::from_secs(2));
        assert_eq!(error["type"], "error");
        assert_eq!(error["fatal"], true);
        assert!(!error["message"].as_str().unwrap().contains("private-input"));
        assert!(!error["message"].as_str().unwrap().contains(&raw));
        drop(plugin.stdin.take());
        let status = plugin.child.wait().unwrap();
        assert!(!status.success());
    }
}

#[test]
fn malformed_and_unterminated_messages_exit_nonzero_without_echoing_input() {
    for unterminated in [false, true] {
        let mut plugin = plugin(&[]);
        if unterminated {
            plugin
                .stdin
                .as_mut()
                .unwrap()
                .write_all(br#"{"type":"hello","protocol":1,"secret":"do-not-echo"}"#)
                .unwrap();
            plugin.stdin.as_mut().unwrap().flush().unwrap();
        } else {
            plugin
                .stdin
                .as_mut()
                .unwrap()
                .write_all(b"not-json\n")
                .unwrap();
        }
        plugin.stdin.take();
        let status = plugin.child.wait().unwrap();
        assert!(!status.success());
    }
}

#[test]
fn oversized_protocol_line_is_rejected_without_diagnostic_stdout() {
    let mut plugin = plugin(&[]);
    let stdin = plugin.stdin.as_mut().unwrap();
    stdin.write_all(&vec![b'x'; 8 * 1024 * 1024 + 1]).unwrap();
    stdin.write_all(b"\n").unwrap();
    stdin.flush().unwrap();
    plugin.stdin.take();
    let status = plugin.child.wait().unwrap();
    assert!(!status.success());
    let error = next(&plugin, Duration::from_millis(500));
    assert_eq!(error["type"], "error");
    assert!(!error["message"].as_str().unwrap().contains('x'));
}
