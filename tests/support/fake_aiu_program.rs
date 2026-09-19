use std::{
    env, fs,
    io::{self, Write},
    thread,
    time::{Duration, Instant},
};

fn quote(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let scenario = args.first().map(String::as_str).unwrap_or("");
    fs::write("started", b"1").unwrap();
    let mut starts = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("started.log")
        .unwrap();
    writeln!(starts, "started").unwrap();
    fs::write("argv.txt", args[1..].join("\n")).unwrap();
    assert_eq!(args.get(1).map(String::as_str), Some("--json"));
    let mut position = 2;
    let provider = if args.get(position).map(String::as_str) == Some("--provider") {
        let provider = args.get(position + 1).expect("provider value").as_str();
        assert!(["claude", "codex"].contains(&provider));
        position += 2;
        provider
    } else {
        "claude"
    };
    let action = args.get(position).expect("AIU action").as_str();
    match action {
        "status" => assert_eq!(args.len(), position + 1),
        "switch" => {
            assert_eq!(args.len(), position + 3);
            assert_eq!(args[position + 1], "--");
            assert!(args[position + 2].starts_with(&format!("{provider}:")));
        }
        _ => panic!("unexpected AIU action"),
    }
    if ["gated", "slow"].contains(&scenario) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !std::path::Path::new("release").exists() {
            assert!(Instant::now() < deadline, "test did not release fake AIU");
            thread::sleep(Duration::from_millis(10));
        }
    }
    if scenario == "fail" {
        eprintln!("fake secret stderr");
        std::process::exit(7);
    }
    if scenario == "malformed" {
        print!("{{bad");
        return;
    }
    if scenario == "oversize" {
        print!("{}", "x".repeat(2_100_000));
        io::stdout().flush().unwrap();
        fs::write("finished", b"1").unwrap();
        return;
    }
    if action == "switch" {
        if scenario == "switch_warn" {
            eprintln!("fake secret warning");
        }
        let target = &args[position + 2];
        let mut switches = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("switch")
            .unwrap();
        writeln!(switches, "switch").unwrap();
        fs::write("switch-target", target).unwrap();
        println!("{{\"switched\":{}}}", quote(target));
    } else if scenario == "stateful" {
        let switched = fs::read_to_string("switch-target").ok();
        let first = "claude:active@example.test";
        let second = "claude:eligible@example.test";
        println!(
            "[{{\"key\":{},\"provider\":\"claude\",\"label\":\"Active\",\"active\":{},\"login\":{{\"state\":\"ok\"}}}},{{\"key\":{},\"provider\":\"claude\",\"label\":\"Eligible\",\"active\":{},\"login\":{{\"state\":\"ok\"}}}}]",
            quote(first),
            switched.is_none() || switched.as_deref() == Some(first),
            quote(second),
            switched.as_deref() == Some(second)
        );
    } else {
        println!(
            "[{{\"key\":{},\"provider\":{},\"label\":\"Test account\",\"login\":{{\"state\":\"ok\"}}}}]",
            quote(&format!("{provider}:a@example.test")),
            quote(provider)
        );
    }
    io::stdout().flush().unwrap();
    fs::write("finished", b"1").unwrap();
}
