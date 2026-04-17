use assert_cmd::Command;
use predicates::str::contains;
use serde_json::Value;

fn kg() -> Command {
    let mut c = Command::cargo_bin("kg").expect("kg binary built");
    c.env_remove("RUST_LOG");
    c
}

fn parse_stdout_json(bytes: &[u8]) -> Value {
    let s = std::str::from_utf8(bytes).expect("stdout is utf-8");
    serde_json::from_str(s.trim_end()).unwrap_or_else(|e| panic!("stdout is not JSON: {e}: {s:?}"))
}

#[test]
fn version_prints_name_and_semver() {
    let assert = kg().arg("--version").assert().success();
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let trimmed = out.trim_end_matches('\n');
    let re = regex_lite("^kg \\d+\\.\\d+\\.\\d+$");
    assert!(re(trimmed), "expected `kg X.Y.Z`, got {trimmed:?}");
}

#[test]
fn help_lists_parse_subcommand() {
    kg().arg("--help")
        .assert()
        .success()
        .stdout(contains("Usage: kg"))
        .stdout(contains("parse"));
}

#[test]
fn unknown_subcommand_emits_envelope_on_stdout() {
    let assert = kg().arg("bogus-cmd").assert().code(2);
    let value = parse_stdout_json(&assert.get_output().stdout);
    assert_eq!(value["ok"], Value::Bool(false));
    assert_eq!(value["error"]["kind"], "unknown_subcommand");
    assert!(value["error"]["message"].is_string());
}

#[test]
fn parse_help_works() {
    kg().args(["parse", "--help"])
        .assert()
        .success()
        .stdout(contains("Usage: kg parse"));
}

#[test]
fn parse_returns_not_implemented_envelope() {
    let assert = kg().arg("parse").assert().code(1);
    let value = parse_stdout_json(&assert.get_output().stdout);
    assert_eq!(value["ok"], Value::Bool(false));
    assert_eq!(value["error"]["kind"], "not_implemented");
    let msg = value["error"]["message"].as_str().expect("message string");
    assert!(msg.contains("parse"), "got {msg:?}");
}

#[test]
fn parse_stdout_is_only_envelope_no_log_lines() {
    let assert = kg().arg("parse").assert().code(1);
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 1, "expected one line on stdout, got {lines:?}");
    let _: Value = serde_json::from_str(lines[0]).expect("only line is JSON");
}

// Tiny regex-lite for the version test: checks `^kg \d+\.\d+\.\d+$` without
// pulling in a regex crate just for one assertion.
fn regex_lite(_pat: &str) -> impl Fn(&str) -> bool {
    |s: &str| {
        let Some(rest) = s.strip_prefix("kg ") else {
            return false;
        };
        let parts: Vec<&str> = rest.split('.').collect();
        parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
    }
}
