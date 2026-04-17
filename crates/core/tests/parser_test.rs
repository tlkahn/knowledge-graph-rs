use std::path::PathBuf;

use kg_core::parser::parse_vault;
use kg_core::types::{ParseEvent, ParsedEdge, ParsedNode};

fn fixture_vault() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vault")
}

fn collect_events() -> Vec<ParseEvent> {
    parse_vault(&fixture_vault()).expect("parse_vault succeeds")
}

fn nodes(events: &[ParseEvent]) -> Vec<&ParsedNode> {
    events
        .iter()
        .filter_map(|e| match e {
            ParseEvent::Node(n) => Some(n),
            _ => None,
        })
        .collect()
}

fn edges(events: &[ParseEvent]) -> Vec<&ParsedEdge> {
    events
        .iter()
        .filter_map(|e| match e {
            ParseEvent::Edge(e) => Some(e),
            _ => None,
        })
        .collect()
}

// --- vault walking ---

#[test]
fn walks_vault_finds_all_md_files() {
    let events = collect_events();
    let ns = nodes(&events);
    assert_eq!(
        ns.len(),
        11,
        "expected 11 nodes, got: {:?}",
        ns.iter().map(|n| &n.id).collect::<Vec<_>>()
    );
}

#[test]
fn excludes_hidden_directories() {
    let events = collect_events();
    let ns = nodes(&events);
    assert!(
        ns.iter().all(|n| !n.id.contains(".obsidian")),
        "should not contain .obsidian nodes"
    );
}

#[test]
fn excludes_non_md_files() {
    let events = collect_events();
    let ns = nodes(&events);
    assert!(
        ns.iter().all(|n| !n.id.contains("photo.png")),
        "should not contain non-md files"
    );
}

#[test]
fn vault_not_found_returns_error() {
    let result = parse_vault(&PathBuf::from("/nonexistent/vault"));
    assert!(result.is_err());
    let err = result.unwrap_err();
    let value = serde_json::to_value(&err).expect("serialize");
    assert_eq!(value["kind"], "vault_not_found");
}

// --- node field correctness ---

#[test]
fn alice_has_correct_frontmatter() {
    let events = collect_events();
    let ns = nodes(&events);
    let alice = ns
        .iter()
        .find(|n| n.id == "People/Alice Smith.md")
        .expect("alice exists");
    assert_eq!(alice.title, "Alice Smith");
    assert!(alice.tags.contains(&"person".to_string()));
    assert!(alice.tags.contains(&"engineer".to_string()));
    assert_eq!(alice.frontmatter["type"], "person");
    assert!(alice.frontmatter["aliases"].is_array());
}

#[test]
fn no_title_falls_back_to_filename() {
    let events = collect_events();
    let ns = nodes(&events);
    let no_title = ns
        .iter()
        .find(|n| n.id == "no-title.md")
        .expect("no-title exists");
    assert_eq!(no_title.title, "no-title");
}

#[test]
fn orphan_has_tags() {
    let events = collect_events();
    let ns = nodes(&events);
    let orphan = ns
        .iter()
        .find(|n| n.id == "orphan.md")
        .expect("orphan exists");
    assert!(orphan.tags.contains(&"misc".to_string()));
}

#[test]
fn first_paragraph_extracted() {
    let events = collect_events();
    let ns = nodes(&events);
    let wt = ns
        .iter()
        .find(|n| n.id == "Concepts/Widget Theory.md")
        .expect("widget theory exists");
    assert!(
        wt.first_paragraph.contains("theoretical framework"),
        "got: {}",
        wt.first_paragraph
    );
}

#[test]
fn malformed_yaml_produces_node_with_empty_frontmatter() {
    let events = collect_events();
    let ns = nodes(&events);
    let malformed = ns
        .iter()
        .find(|n| n.id == "malformed-yaml.md")
        .expect("malformed-yaml exists");
    assert!(
        malformed.frontmatter.as_object().unwrap().is_empty(),
        "expected empty frontmatter, got: {}",
        malformed.frontmatter
    );
}

// --- edge field correctness ---

#[test]
fn edges_from_alice_to_widget_theory() {
    let events = collect_events();
    let es = edges(&events);
    let edge = es
        .iter()
        .find(|e| e.source == "People/Alice Smith.md" && e.target_raw == "Widget Theory")
        .expect("alice -> widget theory edge exists");
    assert!(!edge.context.is_empty(), "context should not be empty");
}

#[test]
fn stub_link_has_raw_target() {
    let events = collect_events();
    let es = edges(&events);
    let stub = es
        .iter()
        .find(|e| e.target_raw == "Nonexistent Page")
        .expect("stub link exists");
    assert_eq!(stub.source, "Ideas/Acme Project.md");
}

#[test]
fn no_edges_from_orphan() {
    let events = collect_events();
    let es = edges(&events);
    assert!(
        es.iter().all(|e| e.source != "orphan.md"),
        "orphan should have no outgoing edges"
    );
}

#[test]
fn code_fence_links_ignored() {
    let events = collect_events();
    let es = edges(&events);
    let code_fence_edges: Vec<_> = es
        .iter()
        .filter(|e| e.source == "code-fences.md")
        .collect();
    assert_eq!(
        code_fence_edges.len(),
        1,
        "expected 1 real edge from code-fences.md, got: {:?}",
        code_fence_edges
    );
    assert_eq!(code_fence_edges[0].target_raw, "Real Link");
}
