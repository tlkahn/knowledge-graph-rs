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

// --- index command tests ---

#[test]
fn index_outputs_valid_json_summary() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("kg-data");

    let assert = kg()
        .args(["index", "--vault", &fixture_vault(), "--data-dir", &data_dir.to_string_lossy()])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let value: Value = serde_json::from_str(stdout.trim()).expect("stdout is JSON");
    assert!(value.get("added").is_some(), "summary should have 'added' field: {value}");
    assert!(value["added"].as_u64().unwrap() > 0);
}

#[test]
fn index_requires_vault() {
    let assert = kg().arg("index").assert().code(1);
    let value = parse_stdout_json(&assert.get_output().stdout);
    assert_eq!(value["ok"], Value::Bool(false));
    assert_eq!(value["error"]["kind"], "vault_not_found");
}

#[test]
fn reindex_shows_zero_changes() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("kg-data");
    let dd = data_dir.to_string_lossy().to_string();

    kg().args(["index", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .success();

    let assert = kg()
        .args(["index", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let value: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(value["added"], 0);
    assert_eq!(value["changed"], 0);
    assert_eq!(value["deleted"], 0);
}

// --- stats command tests ---

#[test]
fn stats_after_index_shows_counts() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("kg-data");
    let dd = data_dir.to_string_lossy().to_string();

    kg().args(["index", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .success();

    let assert = kg()
        .args(["stats", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let value: Value = serde_json::from_str(stdout.trim()).expect("stats is JSON");
    assert!(value["nodes"].as_i64().unwrap() > 0);
    assert!(value["edges"].as_i64().unwrap() > 0);
}

#[test]
fn stats_on_empty_db_shows_zeros() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("kg-data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let dd = data_dir.to_string_lossy().to_string();

    let assert = kg()
        .args(["stats", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let value: Value = serde_json::from_str(stdout.trim()).expect("stats is JSON");
    assert_eq!(value["nodes"], 0);
    assert_eq!(value["edges"], 0);
}

// --- search command tests ---

#[test]
fn search_after_index_returns_results() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("kg-data");
    let dd = data_dir.to_string_lossy().to_string();

    kg().args(["index", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .success();

    let assert = kg()
        .args(["search", "Alice", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    assert!(!lines.is_empty(), "expected search results for 'Alice'");
    for line in &lines {
        let v: Value = serde_json::from_str(line).unwrap_or_else(|e| panic!("bad JSON: {e}: {line}"));
        assert!(v.get("id").is_some());
        assert!(v.get("title").is_some());
        assert!(v.get("score").is_some());
        assert!(v.get("excerpt").is_some());
    }
}

#[test]
fn search_with_limit() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("kg-data");
    let dd = data_dir.to_string_lossy().to_string();

    kg().args(["index", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .success();

    let assert = kg()
        .args(["search", "Alice", "--limit", "1", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    assert!(lines.len() <= 1, "expected at most 1 result with --limit 1, got {}", lines.len());
}

#[test]
fn search_no_matches_empty_output() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("kg-data");
    let dd = data_dir.to_string_lossy().to_string();

    kg().args(["index", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .success();

    let assert = kg()
        .args(["search", "zzzznonexistent", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    assert!(lines.is_empty(), "expected no results");
}

#[test]
fn search_requires_vault() {
    let assert = kg().args(["search", "test"]).assert().code(1);
    let value = parse_stdout_json(&assert.get_output().stdout);
    assert_eq!(value["ok"], Value::Bool(false));
    assert_eq!(value["error"]["kind"], "vault_not_found");
}

// --- neighbors command tests ---

#[test]
fn neighbors_returns_valid_json() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("kg-data");
    let dd = data_dir.to_string_lossy().to_string();

    kg().args(["index", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .success();

    let assert = kg()
        .args(["neighbors", "Concepts/Widget Theory.md", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .success();
    let value = parse_stdout_json(&assert.get_output().stdout);
    assert!(value.is_array());
    let arr = value.as_array().unwrap();
    assert!(!arr.is_empty());
    assert!(arr[0].get("id").is_some());
    assert!(arr[0].get("depth").is_some());
}

#[test]
fn neighbors_nonexistent_id_errors() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("kg-data");
    let dd = data_dir.to_string_lossy().to_string();

    kg().args(["index", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .success();

    let assert = kg()
        .args(["neighbors", "nonexistent.md", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .code(1);
    let value = parse_stdout_json(&assert.get_output().stdout);
    assert_eq!(value["ok"], Value::Bool(false));
    assert_eq!(value["error"]["kind"], "node_not_found");
}

#[test]
fn neighbors_requires_vault() {
    let assert = kg().args(["neighbors", "x"]).assert().code(1);
    let value = parse_stdout_json(&assert.get_output().stdout);
    assert_eq!(value["error"]["kind"], "vault_not_found");
}

// --- path command tests ---

#[test]
fn path_returns_valid_json() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("kg-data");
    let dd = data_dir.to_string_lossy().to_string();

    kg().args(["index", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .success();

    let assert = kg()
        .args(["path", "People/Bob Jones.md", "Ideas/Acme Project.md", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .success();
    let value = parse_stdout_json(&assert.get_output().stdout);
    assert!(value.is_array());
}

#[test]
fn path_nonexistent_id_errors() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("kg-data");
    let dd = data_dir.to_string_lossy().to_string();

    kg().args(["index", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .success();

    let assert = kg()
        .args(["path", "nonexistent.md", "People/Alice Smith.md", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .code(1);
    let value = parse_stdout_json(&assert.get_output().stdout);
    assert_eq!(value["error"]["kind"], "node_not_found");
}

// --- shared command tests ---

#[test]
fn shared_returns_valid_json() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("kg-data");
    let dd = data_dir.to_string_lossy().to_string();

    kg().args(["index", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .success();

    let assert = kg()
        .args(["shared", "People/Alice Smith.md", "People/Bob Jones.md", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .success();
    let value = parse_stdout_json(&assert.get_output().stdout);
    assert!(value.is_array());
}

// --- subgraph command tests ---

#[test]
fn subgraph_returns_valid_json() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("kg-data");
    let dd = data_dir.to_string_lossy().to_string();

    kg().args(["index", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .success();

    let assert = kg()
        .args(["subgraph", "Ideas/Acme Project.md", "--depth", "1", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .success();
    let value = parse_stdout_json(&assert.get_output().stdout);
    assert!(value.get("nodes").is_some());
    assert!(value.get("edges").is_some());
    assert!(value["nodes"].is_array());
    assert!(value["edges"].is_array());
}

// --- rank command tests ---

#[test]
fn rank_returns_valid_json_array() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("kg-data");
    let dd = data_dir.to_string_lossy().to_string();

    kg().args(["index", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .success();

    let assert = kg()
        .args(["rank", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .success();
    let value = parse_stdout_json(&assert.get_output().stdout);
    assert!(value.is_array(), "rank output should be a JSON array");
    let arr = value.as_array().unwrap();
    assert!(!arr.is_empty());
    assert!(arr[0].get("id").is_some());
    assert!(arr[0].get("title").is_some());
    assert!(arr[0].get("score").is_some());
}

#[test]
fn rank_top_limits_results() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("kg-data");
    let dd = data_dir.to_string_lossy().to_string();

    kg().args(["index", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .success();

    let assert = kg()
        .args(["rank", "--top", "2", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .success();
    let value = parse_stdout_json(&assert.get_output().stdout);
    let arr = value.as_array().unwrap();
    assert!(arr.len() <= 2);
}

#[test]
fn rank_sorted_descending() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("kg-data");
    let dd = data_dir.to_string_lossy().to_string();

    kg().args(["index", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .success();

    let assert = kg()
        .args(["rank", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .success();
    let value = parse_stdout_json(&assert.get_output().stdout);
    let arr = value.as_array().unwrap();
    for i in 1..arr.len() {
        let prev = arr[i - 1]["score"].as_f64().unwrap();
        let curr = arr[i]["score"].as_f64().unwrap();
        assert!(prev >= curr, "results should be sorted descending by score");
    }
}

#[test]
fn rank_requires_vault() {
    let assert = kg().arg("rank").assert().code(1);
    let value = parse_stdout_json(&assert.get_output().stdout);
    assert_eq!(value["error"]["kind"], "vault_not_found");
}

#[test]
fn rank_empty_db_returns_empty_array() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("kg-data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let dd = data_dir.to_string_lossy().to_string();

    let assert = kg()
        .args(["rank", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .success();
    let value = parse_stdout_json(&assert.get_output().stdout);
    assert!(value.is_array());
    assert!(value.as_array().unwrap().is_empty());
}

#[test]
fn rank_cached_second_call_matches_first() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("kg-data");
    let dd = data_dir.to_string_lossy().to_string();

    kg().args(["index", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .success();

    let assert1 = kg()
        .args(["rank", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .success();
    let v1 = parse_stdout_json(&assert1.get_output().stdout);

    let assert2 = kg()
        .args(["rank", "--vault", &fixture_vault(), "--data-dir", &dd])
        .assert()
        .success();
    let v2 = parse_stdout_json(&assert2.get_output().stdout);

    let a1 = v1.as_array().unwrap();
    let a2 = v2.as_array().unwrap();
    assert_eq!(a1.len(), a2.len(), "cached call should return same number of entries");
    for (e1, e2) in a1.iter().zip(a2.iter()) {
        assert_eq!(e1["id"], e2["id"], "IDs should match");
        let s1 = e1["score"].as_f64().unwrap();
        let s2 = e2["score"].as_f64().unwrap();
        assert!((s1 - s2).abs() < 1e-10, "scores should match within epsilon");
    }
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
