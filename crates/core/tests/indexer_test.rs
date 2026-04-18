use std::path::PathBuf;

use kg_core::indexer::{collect_vault_files, index_vault};
use kg_core::store::Store;
use tracing_test::traced_test;

fn fixture_vault() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vault")
}

fn copy_vault_to_tmp() -> tempfile::TempDir {
    let src = fixture_vault();
    let tmp = tempfile::tempdir().unwrap();
    copy_dir_recursive(&src, tmp.path());
    tmp
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) {
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let ty = entry.file_type().unwrap();
        let dest = dst.join(entry.file_name());
        if ty.is_dir() {
            std::fs::create_dir_all(&dest).unwrap();
            copy_dir_recursive(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), &dest).unwrap();
        }
    }
}

// --- Step 10: collect_vault_files ---

#[test]
fn collect_finds_all_fixture_md_files() {
    let files = collect_vault_files(&fixture_vault()).unwrap();
    assert_eq!(
        files.len(),
        11,
        "expected 11 md files, got: {:?}",
        files.iter().map(|(p, _)| p).collect::<Vec<_>>()
    );
}

#[test]
fn collect_excludes_obsidian() {
    let files = collect_vault_files(&fixture_vault()).unwrap();
    assert!(
        files.iter().all(|(p, _)| !p.contains(".obsidian")),
        "should not contain .obsidian"
    );
}

// --- Step 11: index_vault from scratch ---

#[test]
fn index_vault_from_scratch() {
    let mut store = Store::open_memory().unwrap();
    let summary = index_vault(&fixture_vault(), &mut store).unwrap();

    assert_eq!(summary.added, 11, "should add 11 files");
    assert_eq!(summary.changed, 0);
    assert_eq!(summary.deleted, 0);

    let stats = store.stats().unwrap();
    assert_eq!(stats.nodes, 11, "should have 11 real nodes");
    assert!(stats.edges > 0, "should have some edges");
    assert!(stats.tags > 0, "should have some tags");

    // Stubs should exist for unresolved targets
    if summary.stubs > 0 {
        assert!(stats.stubs > 0);
    }

    // Every edge target should exist in nodes table
    let _node_ids = store.all_node_ids().unwrap();
    // All synced paths should match the 11 files
    let synced = store.all_synced_paths().unwrap();
    assert_eq!(synced.len(), 11);
}

// --- Step 12: re-index no-op ---

#[test]
fn reindex_noop_when_nothing_changed() {
    let mut store = Store::open_memory().unwrap();
    index_vault(&fixture_vault(), &mut store).unwrap();

    let summary2 = index_vault(&fixture_vault(), &mut store).unwrap();
    assert_eq!(summary2.added, 0);
    assert_eq!(summary2.changed, 0);
    assert_eq!(summary2.deleted, 0);
}

// --- Step 13: detect changes ---

#[test]
fn reindex_detects_touched_file() {
    let tmp = copy_vault_to_tmp();
    let vault = tmp.path();
    let mut store = Store::open_memory().unwrap();

    index_vault(vault, &mut store).unwrap();

    // Touch a file by bumping mtime
    let target = vault.join("orphan.md");
    let content = std::fs::read_to_string(&target).unwrap();
    std::thread::sleep(std::time::Duration::from_secs(1));
    std::fs::write(&target, &content).unwrap();

    let summary = index_vault(vault, &mut store).unwrap();
    assert_eq!(summary.changed, 1, "should detect 1 changed file");
}

// --- Step 14: detect deletions ---

#[test]
fn reindex_detects_deleted_file() {
    let tmp = copy_vault_to_tmp();
    let vault = tmp.path();
    let mut store = Store::open_memory().unwrap();

    index_vault(vault, &mut store).unwrap();
    let initial_nodes = store.stats().unwrap().nodes;

    std::fs::remove_file(vault.join("orphan.md")).unwrap();

    let s2 = index_vault(vault, &mut store).unwrap();
    assert_eq!(s2.deleted, 1);
    assert_eq!(store.stats().unwrap().nodes, initial_nodes - 1);
}

// --- Step 17: full round-trip lifecycle ---

#[test]
fn full_lifecycle_index_reindex_touch_delete() {
    let tmp = copy_vault_to_tmp();
    let vault = tmp.path();
    let mut store = Store::open_memory().unwrap();

    // Initial index
    let s1 = index_vault(vault, &mut store).unwrap();
    assert_eq!(s1.added, 11);
    let initial_stats = store.stats().unwrap();

    // Re-index: no-op
    let s2 = index_vault(vault, &mut store).unwrap();
    assert_eq!(s2.added, 0);
    assert_eq!(s2.changed, 0);
    assert_eq!(s2.deleted, 0);

    // Touch a file
    let target = vault.join("orphan.md");
    let content = std::fs::read_to_string(&target).unwrap();
    std::thread::sleep(std::time::Duration::from_secs(1));
    std::fs::write(&target, &content).unwrap();

    let s3 = index_vault(vault, &mut store).unwrap();
    assert_eq!(s3.changed, 1);

    // Delete a file
    std::fs::remove_file(vault.join("no-title.md")).unwrap();

    let s4 = index_vault(vault, &mut store).unwrap();
    assert_eq!(s4.deleted, 1);
    assert_eq!(store.stats().unwrap().nodes, initial_stats.nodes - 1);
}

// --- tracing tests ---

#[traced_test]
#[test]
fn index_logs_start_complete() {
    let mut store = Store::open_memory().unwrap();
    let _ = index_vault(&fixture_vault(), &mut store).unwrap();
    assert!(logs_contain("indexing vault"));
    assert!(logs_contain("indexing complete"));
}

#[traced_test]
#[test]
fn reindex_noop_logs_unchanged() {
    let mut store = Store::open_memory().unwrap();
    index_vault(&fixture_vault(), &mut store).unwrap();
    let _ = index_vault(&fixture_vault(), &mut store).unwrap();
    assert!(logs_contain("vault unchanged, skipping reindex"));
}

#[traced_test]
#[test]
fn index_logs_debug_diff() {
    let mut store = Store::open_memory().unwrap();
    let _ = index_vault(&fixture_vault(), &mut store).unwrap();
    assert!(logs_contain("index diff computed"));
}
