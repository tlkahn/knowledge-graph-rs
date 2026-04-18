# CLAUDE.md

## What this project is

Rust re-implementation of the `knowledge-graph` TypeScript tool (sibling
repo at `~/Desktop/knowledge-graph`). Reads an Obsidian vault and produces
a queryable knowledge graph. Read-only against the vault, no MCP server.

See `roadmap.md` for the full stage plan (0-8). See
`doc/implementation-notes.md` for detailed learnings and gotchas.

## Commands

```bash
cargo test                      # 244 tests (unit + integration + CLI smoke)
cargo run --bin kg -- --help    # CLI help
cargo run --bin kg -- parse --vault <path>           # stream NDJSON
cargo run --bin kg -- parse --vault <path> --pretty  # envelope JSON
cargo run --bin kg -- resolve "Alice Smith" --vault <path>  # name resolution
cargo run --bin kg -- index --vault <path>           # index vault to SQLite
cargo run --bin kg -- stats --vault <path>           # show graph statistics
cargo run --bin kg -- search "query" --vault <path>  # full-text search (FTS5)
cargo run --bin kg -- search "query" --limit 5 --vault <path>
cargo run --bin kg -- neighbors "People/Alice Smith.md" --vault <path>  # BFS neighbors
cargo run --bin kg -- neighbors "id" --depth 2 --directed --vault <path>
cargo run --bin kg -- path "from" "to" --vault <path>                  # all simple paths
cargo run --bin kg -- shared "a" "b" --vault <path>                    # shared neighbors
cargo run --bin kg -- subgraph "id1" "id2" --depth 1 --vault <path>    # induced subgraph
cargo run --bin kg -- rank --vault <path>                              # PageRank centrality
cargo run --bin kg -- rank --top 10 --vault <path>                     # top-N ranked nodes
```

## Project layout

```
Cargo.toml                  # workspace: crates/core + crates/cli
crates/core/src/
  lib.rs                    # re-exports Error; declares parser, resolve, store, indexer, graph, types, wiki_links
  error.rs                  # Error enum: NotImplemented, Io, VaultNotFound, Database, NodeNotFound
  types.rs                  # ParsedNode, ParsedEdge, ParseEvent, SearchResult, NeighborEntry, Subgraph, RankEntry
  graph.rs                  # KnowledgeGraph: from_store(), neighbors(), path(), shared(), subgraph(), rank(), degree_centrality()
  wiki_links.rs             # RawLink, extract_wiki_links(), strip_code_constructs()
  parser.rs                 # parse_file(), parse_vault() + frontmatter/paragraph helpers
  resolve.rs                # StemLookup, resolve_edges(), resolve_name() — link resolution
  store.rs                  # Store (SQLite): schema, CRUD, queries, Stats, search(), all_edges(), all_nodes_metadata(), get_meta(), set_meta(), max_mtime(), node_titles(), graph_fingerprint()
  indexer.rs                # collect_vault_files(), index_vault() — diff + orchestration
crates/cli/src/
  main.rs                   # entry: tracing init, clap parse, dispatch, exit codes
  cli.rs                    # Cli struct (clap derive): --vault, --data-dir, subcommands
  envelope.rs               # JSON envelope: {"ok":true,"data":...} / {"ok":false,"error":...}
crates/core/tests/
  fixtures/vault/           # 11 .md files + .obsidian/ + attachments/ (excluded by walker)
  parser_test.rs            # integration tests against fixture vault
  store_test.rs             # integration tests for Store (in-memory SQLite)
  indexer_test.rs            # round-trip indexer tests against fixture vault
  graph_test.rs             # integration tests for KnowledgeGraph against fixture vault
crates/cli/tests/
  cli_smoke.rs              # CLI binary tests via assert_cmd
```

## Architecture

Business logic lives in `crates/core` (`kg-core`). The CLI crate is a
thin shell: parse args, call core, format output. Stages add modules to
core and subcommands to CLI in lock-step.

Pipeline (Stages 1-5):
1. `parser::parse_vault()` walks vault via `ignore` crate (skips hidden dirs)
2. Per file: `parser::parse_file()` splits frontmatter via `gray_matter`,
   deserializes YAML into `serde_json::Value`, extracts wiki-links
3. `wiki_links::extract_wiki_links()` strips code constructs then regex-matches
   `[[target]]` / `[[target|alias]]` / `[[target#section|alias]]`
4. Returns `Vec<ParseEvent>` (tagged enum: Node or Edge)
5. `resolve::resolve_edges()` resolves `target_raw` → canonical node IDs via
   `StemLookup` (exact-path → basename-unique → path-suffix disambiguation)
6. `resolve::resolve_name()` provides query-time 5-tier name matching
   (id → exact → case-insensitive → alias → substring)
7. `indexer::index_vault()` orchestrates incremental indexing:
   diff filesystem vs stored mtimes → parse changed → re-resolve all edges →
   persist to SQLite via `store::Store`
8. `store::Store` manages SQLite (6 tables + FTS5: nodes, tags, aliases,
   edges, sync, meta, nodes_fts) with WAL mode. Default DB at
   `<vault>/.kg/kg.db`
9. `store::Store::search()` queries FTS5 with BM25 ranking, snippet
   extraction, and stub exclusion. Schema version 2 adds `tags_text`
   column and `nodes_fts` virtual table with auto-sync triggers
10. `graph::KnowledgeGraph::from_store()` builds a `petgraph::DiGraph`
    from `all_nodes_metadata()` + `all_edges()`, with O(1) ID→NodeIndex
    lookup and stub tracking
11. Four graph traversal operations: `neighbors()` (BFS), `path()` (DFS all
    simple paths), `shared()` (depth-1 intersection), `subgraph()`
    (BFS + induced edges). All default to undirected traversal;
    `--directed` restricts to outgoing edges only
12. `graph::KnowledgeGraph::rank()` runs power-iteration PageRank
    (damping=0.85, max_iter=100, epsilon=1e-6) on the undirected graph
    with isolates removed. Falls back to `degree_centrality()` if
    iteration doesn't converge. Results cached in `meta` table via
    `graph_fingerprint()` so repeated `kg rank` calls are instant

## Conventions

- **Rust 2024 edition**, toolchain 1.94, resolver v3.
- Errors: `kg_core::Error` enum with `thiserror` (variants: NotImplemented,
  Io, VaultNotFound, Database, NodeNotFound). Implements `Serialize` (`kind` + `message`
  fields). CLI wraps in `Envelope` for stdout. `From<rusqlite::Error>` maps
  to `Database`.
- CLI output: always JSON on stdout, logs on stderr. Exit 0/1/2.
- `parse`, `resolve`, and `search` stream bare NDJSON by default;
  `parse --pretty` wraps in an envelope. `index`, `stats`, `neighbors`,
  `path`, `shared`, `subgraph`, and `rank` emit a single JSON object/array.
- Tests: unit tests inline (`#[cfg(test)]`), integration tests in
  `crates/*/tests/`. Fixture vault at `crates/core/tests/fixtures/vault/`.
- `id` = relative path from vault root (e.g. `People/Alice Smith.md`).
  No separate `path` field.
- Inline `#tag` parsing intentionally excluded — tags come from frontmatter only.
- `target_raw` on edges is the unresolved `[[...]]` text. `resolve_edges()`
  turns it into a `ResolvedEdge` with a `LinkResolution` (Resolved / Ambiguous / Unresolved).

## Key dependencies

| Crate | Purpose |
|-------|---------|
| gray_matter 0.3 | frontmatter splitting + YAML parse |
| ignore 0.4 | vault walker (.gitignore-aware, skips hidden) |
| regex 1 | code-block stripping, wiki-link extraction |
| clap 4 | CLI arg parsing (derive + env) |
| serde / serde_json | serialization throughout |
| thiserror 2 | Error enum derive |
| tracing | structured logging to stderr |
| rusqlite 0.35 | SQLite (bundled) for knowledge graph persistence |
| petgraph 0.8 | directed graph for traversal queries (neighbors, paths, subgraph) |
| tempfile 3 | temporary directories for tests (dev-dep) |

## Gotchas for future sessions

- `gray_matter::Matter::<YAML>::parse()` is generic — must annotate the
  result type as `Result<ParsedEntity<serde_json::Value>, _>` or the
  compiler hits E0282. See `parser.rs:74`.
- `serde_yaml` (dtolnay) is archived. Do not add it — `gray_matter`
  bundles `yaml-rust2` internally.
- `Envelope::ok()` is currently unused (both `parse` and `resolve` bypass
  it for streaming). Will be used when a subcommand needs wrapped output.
- The `ignore` walker respects `.gitignore` by default. Test fixtures
  have no `.gitignore` so this is transparent, but be aware if adding
  fixture files that match gitignore patterns.
- `StemLookup` sorts candidates alphabetically in each stem bucket.
  Ambiguous resolution picks the first sorted candidate — this is
  deterministic and tested. Don't change the sort order without updating
  the ambiguity tests.
- `resolve_name` returns at the first tier that produces matches (early
  return). A query matching at the Id tier won't also return Exact or
  Substring hits for the same node.
- Alias extraction from frontmatter handles both `Value::Array` of
  strings and `Value::String` (single alias), mirroring `extract_tags`.
- `Store::open()` runs `PRAGMA journal_mode=WAL` and `foreign_keys=ON`.
  `migrate()` uses `CREATE TABLE IF NOT EXISTS` so re-opening is safe.
  Schema versioning (meta table) drives v1→v2 migration (FTS5).
- FTS5 content-sync mode (`content=nodes`): triggers keep `nodes_fts`
  in sync. `INSERT OR REPLACE` fires DELETE+INSERT triggers correctly.
  `tags_text` is a denormalized space-joined copy of tags for FTS.
- `bm25()` returns negative scores (more negative = better match).
  `search()` sorts ascending so best results come first.
- `snippet()` uses `[`/`]` as highlight markers (no ANSI in JSON output).
  Column index -1 lets FTS5 pick the best matching column.
- `index_vault()` re-parses ALL files for edge resolution even on
  incremental runs (a new/deleted node can change resolution of links
  in unchanged files). Only the diff determines which nodes to upsert.
- Stub node `id` is the raw `target_raw` string, not a `.md` path.
  Stubs have `is_stub=1` and empty title/frontmatter/first_paragraph.
- `replace_all_edges()` deletes ALL edges then re-inserts from the
  resolved set. This is simpler than incremental edge updates.
- The `tempfile` crate is a dev-dep only — tests that mutate vault
  files copy the fixture vault to a tempdir first.
- `KnowledgeGraph` is built once per CLI invocation via
  `from_store(&Store)`. No caching beyond the process lifetime.
- `neighbors()` BFS initializes the visited set with the start node,
  so self-loops don't add the start node as its own neighbor.
- `path()` uses recursive DFS with backtracking. `max_depth` bounds
  the number of edges (path length - 1). Returns only simple paths
  (no repeated nodes). Results sorted lexicographically.
- `shared()` is depth-1 only — intersects immediate neighbor sets.
  Excludes the two query nodes themselves from results.
- `subgraph()` collects BFS neighborhoods from all seeds, then filters
  edges to only those with both endpoints in the included set.
- All four graph traversal operations default to undirected traversal
  (both incoming + outgoing edges). `--directed` restricts to outgoing only.
- Graph query results are deterministically sorted: neighbors by
  (depth, id), paths lexicographically, shared by id, subgraph nodes
  by id and edges by (source, target), rank by descending score.
- `rank()` converts the DiGraph to undirected adjacency (deduped per
  pair), then filters to non-isolate nodes. Self-loops are excluded
  from the undirected view. Scores sum to 1.0.
- `rank()` cache uses `graph_fingerprint()` = `"{nodes+stubs}:{edges}:{max_mtime}"`.
  If fingerprint matches `rank_cache_fingerprint` in meta, cached
  `rank_cache_data` (JSON) is used. Cache is per-process — CLI reads
  from SQLite, computes if stale, writes back.
- `degree_centrality()` normalizes by total degree sum (not N-1),
  so scores also sum to 1.0.
- `cmd_rank()` enriches `RankEntry` with titles from `node_titles()`
  to produce `{id, title, score}` JSON objects.
- `open_store_and_graph()` returns both `Store` and `KnowledgeGraph`
  for commands that need both (currently only `rank`).

## What's next (Stage 7)

Embeddings + semantic search via shell-out (`KG_EMBED_CMD`), with
hybrid FTS+semantic search via reciprocal rank fusion.
