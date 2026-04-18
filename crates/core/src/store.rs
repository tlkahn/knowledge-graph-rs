use std::path::Path;

use rusqlite::Connection;
use serde::Serialize;

use crate::error::Error;
use crate::resolve::{self, ResolvedEdge, LinkResolution};
use crate::types::{ParsedNode, SearchResult};

pub struct Store {
    conn: Connection,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Stats {
    pub nodes: i64,
    pub stubs: i64,
    pub edges: i64,
    pub tags: i64,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, Error> {
        let conn = Connection::open(path)?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    pub fn open_memory() -> Result<Self, Error> {
        let conn = Connection::open_in_memory()?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<(), Error> {
        self.conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        self.conn.execute_batch("PRAGMA foreign_keys=ON;")?;

        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS nodes (
                id TEXT PRIMARY KEY,
                title TEXT,
                first_paragraph TEXT,
                frontmatter JSON,
                mtime INTEGER,
                is_stub INTEGER DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS tags (
                node_id TEXT,
                tag TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_tags_node_id ON tags(node_id);

            CREATE TABLE IF NOT EXISTS aliases (
                node_id TEXT,
                alias TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_aliases_node_id ON aliases(node_id);

            CREATE TABLE IF NOT EXISTS edges (
                source TEXT,
                target TEXT,
                context TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_edges_source ON edges(source);
            CREATE INDEX IF NOT EXISTS idx_edges_target ON edges(target);

            CREATE TABLE IF NOT EXISTS sync (
                path TEXT PRIMARY KEY,
                mtime INTEGER
            );

            CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY,
                value TEXT
            );

            INSERT OR IGNORE INTO meta(key, value) VALUES ('schema_version', '1');",
        )?;

        let version = self.schema_version()?;

        if version < 2 {
            self.conn.execute_batch(
                "ALTER TABLE nodes ADD COLUMN tags_text TEXT DEFAULT '';"
            )?;
            self.conn.execute_batch(
                "UPDATE nodes SET tags_text = COALESCE(
                    (SELECT group_concat(tag, ' ') FROM tags WHERE tags.node_id = nodes.id),
                    ''
                );"
            )?;
            self.conn.execute_batch(
                "UPDATE meta SET value = '2' WHERE key = 'schema_version';"
            )?;
        }

        self.conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS nodes_fts USING fts5(
                title, first_paragraph, tags_text,
                content=nodes, content_rowid=rowid
            );"
        )?;

        self.conn.execute_batch(
            "CREATE TRIGGER IF NOT EXISTS nodes_fts_insert AFTER INSERT ON nodes
            BEGIN
                INSERT INTO nodes_fts(rowid, title, first_paragraph, tags_text)
                VALUES (new.rowid, new.title, new.first_paragraph, new.tags_text);
            END;"
        )?;

        self.conn.execute_batch(
            "CREATE TRIGGER IF NOT EXISTS nodes_fts_delete AFTER DELETE ON nodes
            BEGIN
                INSERT INTO nodes_fts(nodes_fts, rowid, title, first_paragraph, tags_text)
                VALUES ('delete', old.rowid, old.title, old.first_paragraph, old.tags_text);
            END;"
        )?;

        self.conn.execute_batch(
            "CREATE TRIGGER IF NOT EXISTS nodes_fts_update AFTER UPDATE ON nodes
            BEGIN
                INSERT INTO nodes_fts(nodes_fts, rowid, title, first_paragraph, tags_text)
                VALUES ('delete', old.rowid, old.title, old.first_paragraph, old.tags_text);
                INSERT INTO nodes_fts(rowid, title, first_paragraph, tags_text)
                VALUES (new.rowid, new.title, new.first_paragraph, new.tags_text);
            END;"
        )?;

        if version < 2 {
            self.conn.execute_batch(
                "INSERT INTO nodes_fts(nodes_fts) VALUES ('rebuild');"
            )?;
        }

        Ok(())
    }

    pub fn schema_version(&self) -> Result<i64, Error> {
        let version: String = self.conn.query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )?;
        Ok(version.parse::<i64>().unwrap_or(0))
    }

    pub fn upsert_node(&self, node: &ParsedNode, mtime: i64) -> Result<(), Error> {
        let fm_json = serde_json::to_string(&node.frontmatter).unwrap_or_default();
        let tags_text = node.tags.join(" ");

        self.conn.execute(
            "INSERT OR REPLACE INTO nodes(id, title, first_paragraph, frontmatter, mtime, is_stub, tags_text)
             VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6)",
            rusqlite::params![node.id, node.title, node.first_paragraph, fm_json, mtime, tags_text],
        )?;

        self.conn
            .execute("DELETE FROM tags WHERE node_id = ?1", [&node.id])?;
        for tag in &node.tags {
            self.conn.execute(
                "INSERT INTO tags(node_id, tag) VALUES (?1, ?2)",
                rusqlite::params![node.id, tag],
            )?;
        }

        self.conn
            .execute("DELETE FROM aliases WHERE node_id = ?1", [&node.id])?;
        let aliases = resolve::extract_aliases(&node.frontmatter);
        for alias in &aliases {
            self.conn.execute(
                "INSERT INTO aliases(node_id, alias) VALUES (?1, ?2)",
                rusqlite::params![node.id, alias],
            )?;
        }

        self.conn.execute(
            "INSERT OR REPLACE INTO sync(path, mtime) VALUES (?1, ?2)",
            rusqlite::params![node.id, mtime],
        )?;

        Ok(())
    }

    pub fn upsert_stub(&self, id: &str) -> Result<(), Error> {
        self.conn.execute(
            "INSERT OR IGNORE INTO nodes(id, title, first_paragraph, frontmatter, mtime, is_stub)
             VALUES (?1, '', '', '{}', 0, 1)",
            [id],
        )?;
        Ok(())
    }

    pub fn delete_node(&self, id: &str) -> Result<(), Error> {
        self.conn.execute("DELETE FROM nodes WHERE id = ?1", [id])?;
        self.conn
            .execute("DELETE FROM tags WHERE node_id = ?1", [id])?;
        self.conn
            .execute("DELETE FROM aliases WHERE node_id = ?1", [id])?;
        self.conn
            .execute("DELETE FROM edges WHERE source = ?1 OR target = ?1", [id])?;
        self.conn
            .execute("DELETE FROM sync WHERE path = ?1", [id])?;
        Ok(())
    }

    pub fn insert_edge(&self, source: &str, target: &str, ctx: &str) -> Result<(), Error> {
        self.conn.execute(
            "INSERT INTO edges(source, target, context) VALUES (?1, ?2, ?3)",
            rusqlite::params![source, target, ctx],
        )?;
        Ok(())
    }

    pub fn delete_edges_from(&self, source: &str) -> Result<(), Error> {
        self.conn
            .execute("DELETE FROM edges WHERE source = ?1", [source])?;
        Ok(())
    }

    pub fn replace_all_edges(&self, edges: &[ResolvedEdge]) -> Result<(), Error> {
        self.conn.execute("DELETE FROM edges", [])?;
        for edge in edges {
            let target = match &edge.resolution {
                LinkResolution::Resolved { id } => id.as_str(),
                LinkResolution::Ambiguous { picked, .. } => picked.as_str(),
                LinkResolution::Unresolved => &edge.target_raw,
            };
            self.conn.execute(
                "INSERT INTO edges(source, target, context) VALUES (?1, ?2, ?3)",
                rusqlite::params![edge.source, target, edge.context],
            )?;
        }
        Ok(())
    }

    pub fn get_sync_mtime(&self, path: &str) -> Result<Option<i64>, Error> {
        let mut stmt = self.conn.prepare("SELECT mtime FROM sync WHERE path = ?1")?;
        let mut rows = stmt.query([path])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    pub fn all_synced_paths(&self) -> Result<Vec<String>, Error> {
        let mut stmt = self.conn.prepare("SELECT path FROM sync ORDER BY path")?;
        let paths = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(paths)
    }

    pub fn all_node_ids(&self) -> Result<Vec<String>, Error> {
        let mut stmt = self.conn.prepare("SELECT id FROM nodes ORDER BY id")?;
        let ids = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(ids)
    }

    pub fn stats(&self) -> Result<Stats, Error> {
        let nodes: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE is_stub = 0",
            [],
            |row| row.get(0),
        )?;
        let stubs: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE is_stub = 1",
            [],
            |row| row.get(0),
        )?;
        let edges: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM edges", [], |row| row.get(0))?;
        let tags: i64 = self.conn.query_row(
            "SELECT COUNT(DISTINCT tag) FROM tags",
            [],
            |row| row.get(0),
        )?;
        Ok(Stats {
            nodes,
            stubs,
            edges,
            tags,
        })
    }

    pub fn search(&self, query: &str, limit: i64) -> Result<Vec<SearchResult>, Error> {
        let mut stmt = self.conn.prepare(
            "SELECT n.id, n.title, bm25(nodes_fts) AS score,
                    snippet(nodes_fts, -1, '[', ']', '...', 64) AS excerpt
             FROM nodes_fts
             JOIN nodes n ON n.rowid = nodes_fts.rowid
             WHERE nodes_fts MATCH ?1 AND n.is_stub = 0
             ORDER BY score
             LIMIT ?2"
        )?;

        let results = stmt.query_map(rusqlite::params![query, limit], |row| {
            Ok(SearchResult {
                id: row.get(0)?,
                title: row.get(1)?,
                score: row.get(2)?,
                excerpt: row.get(3)?,
            })
        })?.collect::<Result<Vec<_>, _>>()?;

        Ok(results)
    }

    pub fn begin_transaction(&self) -> Result<(), Error> {
        self.conn.execute_batch("BEGIN")?;
        Ok(())
    }

    pub fn commit(&self) -> Result<(), Error> {
        self.conn.execute_batch("COMMIT")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_node(id: &str, title: &str, tags: &[&str], fm: serde_json::Value) -> ParsedNode {
        ParsedNode {
            id: id.into(),
            title: title.into(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            frontmatter: fm,
            first_paragraph: format!("First paragraph of {title}"),
        }
    }

    // --- Step 2: open + schema ---

    #[test]
    fn open_memory_creates_tables() {
        let store = Store::open_memory().expect("open_memory");
        let tables: Vec<String> = {
            let mut stmt = store
                .conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
                .unwrap();
            stmt.query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        assert!(tables.contains(&"nodes".to_string()));
        assert!(tables.contains(&"tags".to_string()));
        assert!(tables.contains(&"aliases".to_string()));
        assert!(tables.contains(&"edges".to_string()));
        assert!(tables.contains(&"sync".to_string()));
        assert!(tables.contains(&"meta".to_string()));
    }

    #[test]
    fn schema_version_is_2() {
        let store = Store::open_memory().unwrap();
        assert_eq!(store.schema_version().unwrap(), 2);
    }

    #[test]
    fn fts_table_exists() {
        let store = Store::open_memory().unwrap();
        let count: i64 = store.conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='nodes_fts'",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn open_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        {
            let store = Store::open(&db_path).unwrap();
            assert_eq!(store.schema_version().unwrap(), 2);
        }
        {
            let store = Store::open(&db_path).unwrap();
            assert_eq!(store.schema_version().unwrap(), 2);
        }
    }

    // --- Step 3: upsert_node ---

    #[test]
    fn upsert_node_insert_and_readback() {
        let store = Store::open_memory().unwrap();
        let node = make_node("People/Alice.md", "Alice", &["person"], json!({"title": "Alice"}));
        store.upsert_node(&node, 1000).unwrap();

        let title: String = store
            .conn
            .query_row("SELECT title FROM nodes WHERE id = ?1", ["People/Alice.md"], |r| r.get(0))
            .unwrap();
        assert_eq!(title, "Alice");
    }

    #[test]
    fn upsert_node_writes_tags() {
        let store = Store::open_memory().unwrap();
        let node = make_node("a.md", "A", &["tag1", "tag2"], json!({}));
        store.upsert_node(&node, 1).unwrap();

        let mut stmt = store.conn.prepare("SELECT tag FROM tags WHERE node_id = 'a.md' ORDER BY tag").unwrap();
        let tags: Vec<String> = stmt.query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
        assert_eq!(tags, vec!["tag1", "tag2"]);
    }

    #[test]
    fn upsert_node_writes_aliases() {
        let store = Store::open_memory().unwrap();
        let node = make_node("a.md", "A", &[], json!({"aliases": ["Alpha", "Alfa"]}));
        store.upsert_node(&node, 1).unwrap();

        let mut stmt = store.conn.prepare("SELECT alias FROM aliases WHERE node_id = 'a.md' ORDER BY alias").unwrap();
        let aliases: Vec<String> = stmt.query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
        assert_eq!(aliases, vec!["Alfa", "Alpha"]);
    }

    #[test]
    fn upsert_node_writes_sync() {
        let store = Store::open_memory().unwrap();
        let node = make_node("a.md", "A", &[], json!({}));
        store.upsert_node(&node, 42).unwrap();

        assert_eq!(store.get_sync_mtime("a.md").unwrap(), Some(42));
    }

    #[test]
    fn upsert_node_replaces_on_conflict() {
        let store = Store::open_memory().unwrap();
        let node1 = make_node("a.md", "Old", &[], json!({}));
        store.upsert_node(&node1, 1).unwrap();
        let node2 = make_node("a.md", "New", &[], json!({}));
        store.upsert_node(&node2, 2).unwrap();

        let title: String = store.conn.query_row("SELECT title FROM nodes WHERE id = 'a.md'", [], |r| r.get(0)).unwrap();
        assert_eq!(title, "New");
    }

    #[test]
    fn upsert_node_replaces_tags_on_reupsert() {
        let store = Store::open_memory().unwrap();
        let node1 = make_node("a.md", "A", &["old"], json!({}));
        store.upsert_node(&node1, 1).unwrap();
        let node2 = make_node("a.md", "A", &["new1", "new2"], json!({}));
        store.upsert_node(&node2, 2).unwrap();

        let mut stmt = store.conn.prepare("SELECT tag FROM tags WHERE node_id = 'a.md' ORDER BY tag").unwrap();
        let tags: Vec<String> = stmt.query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
        assert_eq!(tags, vec!["new1", "new2"]);
    }

    // --- Step 4: delete_node ---

    #[test]
    fn delete_node_removes_all_data() {
        let store = Store::open_memory().unwrap();
        let node = make_node("a.md", "A", &["t"], json!({"aliases": ["X"]}));
        store.upsert_node(&node, 1).unwrap();
        store.insert_edge("a.md", "b.md", "ctx").unwrap();

        store.delete_node("a.md").unwrap();

        let count: i64 = store.conn.query_row("SELECT COUNT(*) FROM nodes WHERE id = 'a.md'", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0);
        let tag_count: i64 = store.conn.query_row("SELECT COUNT(*) FROM tags WHERE node_id = 'a.md'", [], |r| r.get(0)).unwrap();
        assert_eq!(tag_count, 0);
        let alias_count: i64 = store.conn.query_row("SELECT COUNT(*) FROM aliases WHERE node_id = 'a.md'", [], |r| r.get(0)).unwrap();
        assert_eq!(alias_count, 0);
        let sync_count: i64 = store.conn.query_row("SELECT COUNT(*) FROM sync WHERE path = 'a.md'", [], |r| r.get(0)).unwrap();
        assert_eq!(sync_count, 0);
    }

    #[test]
    fn delete_node_removes_edges_from_and_to() {
        let store = Store::open_memory().unwrap();
        let node_a = make_node("a.md", "A", &[], json!({}));
        let node_b = make_node("b.md", "B", &[], json!({}));
        store.upsert_node(&node_a, 1).unwrap();
        store.upsert_node(&node_b, 1).unwrap();
        store.insert_edge("a.md", "b.md", "").unwrap();
        store.insert_edge("b.md", "a.md", "").unwrap();

        store.delete_node("a.md").unwrap();

        let count: i64 = store.conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn delete_node_noop_for_nonexistent() {
        let store = Store::open_memory().unwrap();
        store.delete_node("nonexistent.md").unwrap();
    }

    // --- Step 5: edge operations ---

    #[test]
    fn insert_edge_and_query() {
        let store = Store::open_memory().unwrap();
        store.insert_edge("a.md", "b.md", "links to b").unwrap();

        let ctx: String = store.conn.query_row(
            "SELECT context FROM edges WHERE source = 'a.md' AND target = 'b.md'",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(ctx, "links to b");
    }

    #[test]
    fn delete_edges_from_source() {
        let store = Store::open_memory().unwrap();
        store.insert_edge("a.md", "b.md", "").unwrap();
        store.insert_edge("a.md", "c.md", "").unwrap();
        store.insert_edge("x.md", "y.md", "").unwrap();

        store.delete_edges_from("a.md").unwrap();

        let count: i64 = store.conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn replace_all_edges_clears_and_inserts() {
        let store = Store::open_memory().unwrap();
        store.insert_edge("old.md", "old_target.md", "").unwrap();

        let resolved = vec![
            ResolvedEdge {
                source: "a.md".into(),
                target_raw: "B".into(),
                context: "link to B".into(),
                resolution: LinkResolution::Resolved { id: "b.md".into() },
            },
            ResolvedEdge {
                source: "a.md".into(),
                target_raw: "Ghost".into(),
                context: "link to Ghost".into(),
                resolution: LinkResolution::Unresolved,
            },
        ];
        store.replace_all_edges(&resolved).unwrap();

        let count: i64 = store.conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 2);

        let target: String = store.conn.query_row(
            "SELECT target FROM edges WHERE source = 'a.md' AND context = 'link to B'",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(target, "b.md");

        let unresolved_target: String = store.conn.query_row(
            "SELECT target FROM edges WHERE source = 'a.md' AND context = 'link to Ghost'",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(unresolved_target, "Ghost");
    }

    // --- Step 6: upsert_stub ---

    #[test]
    fn upsert_stub_creates_stub() {
        let store = Store::open_memory().unwrap();
        store.upsert_stub("Ghost").unwrap();

        let is_stub: i64 = store.conn.query_row(
            "SELECT is_stub FROM nodes WHERE id = 'Ghost'",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(is_stub, 1);
    }

    #[test]
    fn upsert_stub_does_not_overwrite_real_node() {
        let store = Store::open_memory().unwrap();
        let node = make_node("Real.md", "Real", &[], json!({}));
        store.upsert_node(&node, 1).unwrap();

        store.upsert_stub("Real.md").unwrap();

        let is_stub: i64 = store.conn.query_row(
            "SELECT is_stub FROM nodes WHERE id = 'Real.md'",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(is_stub, 0);
    }

    #[test]
    fn upsert_stub_idempotent() {
        let store = Store::open_memory().unwrap();
        store.upsert_stub("Ghost").unwrap();
        store.upsert_stub("Ghost").unwrap();

        let count: i64 = store.conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE id = 'Ghost'",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(count, 1);
    }

    // --- Step 7: sync queries ---

    #[test]
    fn get_sync_mtime_none_for_unknown() {
        let store = Store::open_memory().unwrap();
        assert_eq!(store.get_sync_mtime("unknown.md").unwrap(), None);
    }

    #[test]
    fn get_sync_mtime_returns_stored() {
        let store = Store::open_memory().unwrap();
        let node = make_node("a.md", "A", &[], json!({}));
        store.upsert_node(&node, 999).unwrap();
        assert_eq!(store.get_sync_mtime("a.md").unwrap(), Some(999));
    }

    #[test]
    fn all_synced_paths_returns_all() {
        let store = Store::open_memory().unwrap();
        let node_a = make_node("b.md", "B", &[], json!({}));
        let node_b = make_node("a.md", "A", &[], json!({}));
        store.upsert_node(&node_a, 1).unwrap();
        store.upsert_node(&node_b, 1).unwrap();

        let paths = store.all_synced_paths().unwrap();
        assert_eq!(paths, vec!["a.md", "b.md"]);
    }

    // --- Step 8: stats ---

    #[test]
    fn stats_empty_db() {
        let store = Store::open_memory().unwrap();
        let s = store.stats().unwrap();
        assert_eq!(s, Stats { nodes: 0, stubs: 0, edges: 0, tags: 0 });
    }

    #[test]
    fn stats_populated() {
        let store = Store::open_memory().unwrap();
        let node = make_node("a.md", "A", &["t1", "t2"], json!({}));
        store.upsert_node(&node, 1).unwrap();
        let node2 = make_node("b.md", "B", &["t1"], json!({}));
        store.upsert_node(&node2, 1).unwrap();
        store.upsert_stub("Ghost").unwrap();
        store.insert_edge("a.md", "b.md", "").unwrap();
        store.insert_edge("a.md", "Ghost", "").unwrap();

        let s = store.stats().unwrap();
        assert_eq!(s.nodes, 2);
        assert_eq!(s.stubs, 1);
        assert_eq!(s.edges, 2);
        assert_eq!(s.tags, 2); // distinct: t1, t2
    }

    // --- FTS5 / search tests ---

    #[test]
    fn upsert_node_populates_tags_text() {
        let store = Store::open_memory().unwrap();
        let node = make_node("a.md", "A", &["rust", "coding"], json!({}));
        store.upsert_node(&node, 1).unwrap();

        let tags_text: String = store.conn.query_row(
            "SELECT tags_text FROM nodes WHERE id = 'a.md'",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(tags_text, "rust coding");
    }

    #[test]
    fn fts_trigger_fires_on_upsert() {
        let store = Store::open_memory().unwrap();
        let node = make_node("a.md", "Alpha", &[], json!({}));
        store.upsert_node(&node, 1).unwrap();

        let count: i64 = store.conn.query_row(
            "SELECT COUNT(*) FROM nodes_fts WHERE nodes_fts MATCH 'Alpha'",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn search_by_title() {
        let store = Store::open_memory().unwrap();
        let node = make_node("People/Alice.md", "Alice Smith", &["person"], json!({}));
        store.upsert_node(&node, 1).unwrap();

        let results = store.search("Alice", 20).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "People/Alice.md");
        assert_eq!(results[0].title, "Alice Smith");
        assert!(results[0].score < 0.0);
        assert!(!results[0].excerpt.is_empty());
    }

    #[test]
    fn search_by_tag() {
        let store = Store::open_memory().unwrap();
        let node = make_node("a.md", "Something", &["engineering", "rust"], json!({}));
        store.upsert_node(&node, 1).unwrap();

        let results = store.search("engineering", 20).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "a.md");
    }

    #[test]
    fn search_by_paragraph() {
        let store = Store::open_memory().unwrap();
        let mut node = make_node("a.md", "Title", &[], json!({}));
        node.first_paragraph = "Quantum computing revolutionizes cryptography".into();
        store.upsert_node(&node, 1).unwrap();

        let results = store.search("cryptography", 20).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "a.md");
    }

    #[test]
    fn search_bm25_ordering() {
        let store = Store::open_memory().unwrap();
        let mut strong = make_node("strong.md", "Rust Rust Rust", &["rust"], json!({}));
        strong.first_paragraph = "Rust programming language for systems".into();
        store.upsert_node(&strong, 1).unwrap();

        let mut weak = make_node("weak.md", "Other Topic", &[], json!({}));
        weak.first_paragraph = "Mentions rust once in passing".into();
        store.upsert_node(&weak, 1).unwrap();

        let results = store.search("rust", 20).unwrap();
        assert!(results.len() >= 2);
        assert_eq!(results[0].id, "strong.md", "strongly relevant doc should rank first");
    }

    #[test]
    fn search_excludes_stubs() {
        let store = Store::open_memory().unwrap();
        store.upsert_stub("Ghost Node").unwrap();

        let results = store.search("Ghost", 20).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn search_respects_limit() {
        let store = Store::open_memory().unwrap();
        for i in 0..5 {
            let node = make_node(&format!("{i}.md"), &format!("Searchable Item {i}"), &[], json!({}));
            store.upsert_node(&node, 1).unwrap();
        }

        let results = store.search("Searchable", 2).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn search_no_matches_returns_empty() {
        let store = Store::open_memory().unwrap();
        let node = make_node("a.md", "Alpha", &[], json!({}));
        store.upsert_node(&node, 1).unwrap();

        let results = store.search("zzzznonexistent", 20).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn fts_trigger_fires_on_delete() {
        let store = Store::open_memory().unwrap();
        let node = make_node("a.md", "Deleteable", &[], json!({}));
        store.upsert_node(&node, 1).unwrap();

        store.delete_node("a.md").unwrap();

        let results = store.search("Deleteable", 20).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn fts_trigger_fires_on_update() {
        let store = Store::open_memory().unwrap();
        let old = make_node("a.md", "OldTitle", &[], json!({}));
        store.upsert_node(&old, 1).unwrap();

        let new = make_node("a.md", "NewTitle", &[], json!({}));
        store.upsert_node(&new, 2).unwrap();

        let old_results = store.search("OldTitle", 20).unwrap();
        assert!(old_results.is_empty(), "old title should not be in FTS");

        let new_results = store.search("NewTitle", 20).unwrap();
        assert_eq!(new_results.len(), 1);
        assert_eq!(new_results[0].title, "NewTitle");
    }

    #[test]
    fn v1_to_v2_migration_backfills_tags_text() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        // Simulate a v1 database
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
            conn.execute_batch(
                "CREATE TABLE nodes (
                    id TEXT PRIMARY KEY,
                    title TEXT,
                    first_paragraph TEXT,
                    frontmatter JSON,
                    mtime INTEGER,
                    is_stub INTEGER DEFAULT 0
                );
                CREATE TABLE tags (node_id TEXT, tag TEXT);
                CREATE TABLE aliases (node_id TEXT, alias TEXT);
                CREATE TABLE edges (source TEXT, target TEXT, context TEXT);
                CREATE TABLE sync (path TEXT PRIMARY KEY, mtime INTEGER);
                CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);
                INSERT INTO meta(key, value) VALUES ('schema_version', '1');
                INSERT INTO nodes(id, title, first_paragraph, frontmatter, mtime, is_stub)
                    VALUES ('a.md', 'Alpha', 'First paragraph', '{}', 1, 0);
                INSERT INTO tags(node_id, tag) VALUES ('a.md', 'rust');
                INSERT INTO tags(node_id, tag) VALUES ('a.md', 'coding');",
            ).unwrap();
        }

        // Open with Store — should trigger v1→v2 migration
        let store = Store::open(&db_path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 2);

        let tags_text: String = store.conn.query_row(
            "SELECT tags_text FROM nodes WHERE id = 'a.md'",
            [],
            |r| r.get(0),
        ).unwrap();
        assert!(!tags_text.is_empty(), "tags_text should be backfilled");

        let results = store.search("Alpha", 20).unwrap();
        assert_eq!(results.len(), 1);
    }
}
