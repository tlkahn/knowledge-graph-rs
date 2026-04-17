use std::path::PathBuf;

use assert_cmd::Command;
use predicates::str::contains;
use serde_json::Value;

fn kg() -> Command {
    let mut c = Command::cargo_bin("kg").expect("kg binary built");
    c.env_remove("RUST_LOG");
    c.env_remove("KG_VAULT_PATH");
    c
}

fn parse_stdout_json(bytes: &[u8]) -> Value {
    let s = std::str::from_utf8(bytes).expect("stdout is utf-8");
    serde_json::from_str(s.trim_end()).unwrap_or_else(|e| panic!("stdout is not JSON: {e}: {s:?}"))
}

fn fixture_vault() -> String {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .unwrap()
        .join("core/tests/fixtures/vault")
        .to_string_lossy()
        .to_string()
}

// --- existing smoke tests ---

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

// --- parse command tests ---

#[test]
fn parse_requires_vault_path() {
    let assert = kg().arg("parse").assert().code(1);
    let value = parse_stdout_json(&assert.get_output().stdout);
    assert_eq!(value["ok"], Value::Bool(false));
    assert_eq!(value["error"]["kind"], "vault_not_found");
}

#[test]
fn parse_without_vault_emits_single_line() {
    let assert = kg().arg("parse").assert().code(1);
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 1, "expected one line on stdout, got {lines:?}");
    let _: Value = serde_json::from_str(lines[0]).expect("only line is JSON");
}

#[test]
fn parse_streams_ndjson() {
    let assert = kg()
        .args(["parse", "--vault", &fixture_vault()])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    assert!(lines.len() > 1, "expected multiple NDJSON lines");
    for line in &lines {
        let v: Value = serde_json::from_str(line).unwrap_or_else(|e| panic!("bad JSON: {e}: {line}"));
        assert!(
            v.get("type").is_some(),
            "each line must have a \"type\" field: {v}"
        );
    }
}

#[test]
fn parse_pretty_outputs_envelope() {
    let assert = kg()
        .args(["parse", "--vault", &fixture_vault(), "--pretty"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let value: Value = serde_json::from_str(&stdout).expect("stdout is JSON");
    assert_eq!(value["ok"], Value::Bool(true));
    assert!(value["data"].is_array(), "data should be array");
}

#[test]
fn parse_nonexistent_vault_returns_error() {
    let assert = kg()
        .args(["parse", "--vault", "/nonexistent/vault/path"])
        .assert()
        .code(1);
    let value = parse_stdout_json(&assert.get_output().stdout);
    assert_eq!(value["ok"], Value::Bool(false));
    assert_eq!(value["error"]["kind"], "vault_not_found");
}

// --- resolve command tests ---

#[test]
fn resolve_finds_alice_smith() {
    let assert = kg()
        .args(["resolve", "Alice Smith", "--vault", &fixture_vault()])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    assert!(!lines.is_empty(), "expected at least one match");
    for line in &lines {
        let v: Value = serde_json::from_str(line).unwrap_or_else(|e| panic!("bad JSON: {e}: {line}"));
        assert!(v.get("id").is_some());
        assert!(v.get("kind").is_some());
    }
}

#[test]
fn resolve_nonexistent_name_empty_output() {
    let assert = kg()
        .args(["resolve", "NonExistentXYZ", "--vault", &fixture_vault()])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    assert!(lines.is_empty(), "expected no matches, got: {lines:?}");
}

#[test]
fn resolve_requires_vault_path() {
    let assert = kg().args(["resolve", "Alice"]).assert().code(1);
    let value = parse_stdout_json(&assert.get_output().stdout);
    assert_eq!(value["ok"], Value::Bool(false));
    assert_eq!(value["error"]["kind"], "vault_not_found");
}

fn regex_lite(_pat: &str) -> impl Fn(&str) -> bool {
    |s: &str| {
        let Some(rest) = s.strip_prefix("kg ") else {
            return false;
        };
        let parts: Vec<&str> = rest.split('.').collect();
        parts.len() == 3
            && parts
                .iter()
                .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
    }
}
