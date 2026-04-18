use std::path::PathBuf;

use kg_core::graph::KnowledgeGraph;
use kg_core::indexer::index_vault;
use kg_core::store::Store;

fn fixture_vault() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vault")
}

fn build_fixture_graph() -> KnowledgeGraph {
    let mut store = Store::open_memory().unwrap();
    index_vault(&fixture_vault(), &mut store).unwrap();
    KnowledgeGraph::from_store(&store).unwrap()
}

#[test]
fn graph_counts() {
    let kg = build_fixture_graph();
    assert_eq!(kg.node_count(), 13, "11 real nodes + 2 stubs");
    assert_eq!(kg.edge_count(), 21);
}

#[test]
fn neighbors_widget_theory() {
    let kg = build_fixture_graph();
    let result = kg.neighbors("Concepts/Widget Theory.md", 1, false).unwrap();
    let ids: Vec<&str> = result.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids.len(), 5);
    assert!(ids.contains(&"Archive/Alice Smith.md"));
    assert!(ids.contains(&"Concepts/Gadget Pattern.md"));
    assert!(ids.contains(&"Ideas/Acme Project.md"));
    assert!(ids.contains(&"People/Alice Smith.md"));
    assert!(ids.contains(&"People/Bob Jones.md"));
}

#[test]
fn neighbors_orphan_is_isolated() {
    let kg = build_fixture_graph();
    let result = kg.neighbors("orphan.md", 3, false).unwrap();
    assert!(result.is_empty());
}

#[test]
fn path_bob_to_acme() {
    let kg = build_fixture_graph();
    let paths = kg.path("People/Bob Jones.md", "Ideas/Acme Project.md", 5, false).unwrap();
    assert!(!paths.is_empty(), "should find at least one path");
    let shortest = paths.iter().map(|p| p.len()).min().unwrap();
    assert!(shortest <= 6, "shortest path should be reasonable, got {shortest} nodes");
    for p in &paths {
        assert!(*p.first().unwrap() == "People/Bob Jones.md");
        assert!(*p.last().unwrap() == "Ideas/Acme Project.md");
    }
}

#[test]
fn path_orphan_to_alice_is_empty() {
    let kg = build_fixture_graph();
    let paths = kg.path("orphan.md", "People/Alice Smith.md", 5, false).unwrap();
    assert!(paths.is_empty());
}

#[test]
fn shared_alice_bob() {
    let kg = build_fixture_graph();
    let common = kg.shared("People/Alice Smith.md", "People/Bob Jones.md", false).unwrap();
    assert!(common.contains(&"Concepts/Widget Theory.md".to_string()));
}

#[test]
fn subgraph_acme_depth1_includes_stub() {
    let kg = build_fixture_graph();
    let sg = kg.subgraph(&["Ideas/Acme Project.md"], 1, false).unwrap();
    let stub = sg.nodes.iter().find(|n| n.id == "Nonexistent Page");
    assert!(stub.is_some(), "should include stub 'Nonexistent Page'");
    assert!(stub.unwrap().is_stub);
}

#[test]
fn neighbors_code_fences() {
    let kg = build_fixture_graph();
    let result = kg.neighbors("code-fences.md", 1, false).unwrap();
    let ids: Vec<&str> = result.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, vec!["Real Link"]);
}
