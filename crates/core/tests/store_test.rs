use kg_core::store::Store;

#[test]
fn open_memory_and_schema_version() {
    let store = Store::open_memory().expect("open_memory");
    assert_eq!(store.schema_version().unwrap(), 2);
}

#[test]
fn open_file_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");
    {
        let s = Store::open(&db).unwrap();
        assert_eq!(s.schema_version().unwrap(), 2);
    }
    {
        let s = Store::open(&db).unwrap();
        assert_eq!(s.schema_version().unwrap(), 2);
    }
}

#[test]
fn stats_on_empty_db() {
    let store = Store::open_memory().unwrap();
    let s = store.stats().unwrap();
    assert_eq!(s.nodes, 0);
    assert_eq!(s.stubs, 0);
    assert_eq!(s.edges, 0);
    assert_eq!(s.tags, 0);
}

#[test]
fn search_on_empty_db() {
    let store = Store::open_memory().unwrap();
    let results = store.search("anything", 20).unwrap();
    assert!(results.is_empty());
}
