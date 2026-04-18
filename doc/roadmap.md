# Rust Re-implementation Roadmap

Clean-slate Rust port of the core, non-UI features of `knowledge-graph`
(Typescript).
Scope: library crate + single CLI binary. Read-only against the vault. No MCP.

## Design decisions (locked)

- **Blueprint**: Refer to the original **blueprint project**
  [knowledge-graph](~/Desktop/knowledge-graph/CLAUDE.md)
- **Layout**: Cargo workspace at `~/Desktop/knowledge-graph-rs/`.
  - `crates/core` — library (`kg-core`): pure logic, no I/O glue.
  - `crates/cli` — binary (`kg`): argument parsing, JSON-on-stdout, exit codes.
- **Schema**: clean-slate SQLite. No compat with the TS `kg.db`.
- **Storage**: `rusqlite` + FTS5 for keyword search. Vectors as plain `BLOB`
  with brute-force cosine in Rust for MVP (upgrade to `sqlite-vec` only if a
  vault exceeds ~50k nodes).
- **Embeddings**: shell-out. The binary invokes a user-configured command
  (`KG_EMBED_CMD`), pipes JSON `{"texts":[...]}` on stdin, expects JSON
  `{"vectors":[[...],...]}` on stdout. No embedder is bundled.
- **Read-only**: no `writer.ts` equivalent. The tool never mutates the vault.
- **Composition**: one binary, subcommands, every subcommand emits
  line-delimited or single-document JSON on stdout so results pipe into `jq`
  or another `kg` invocation.
- **Graph library**: `petgraph`.

## Open questions to revisit later

- Do we ever want Louvain / betweenness? (Deferred — Stage 8, optional.)
- When does brute-force cosine stop being acceptable? (Benchmark at Stage 6.)
- Do we want a stable JSON schema versioned across releases, or keep it fluid
  until the surface stabilizes? (Assume fluid until Stage 5.)

---

## Stages

Each stage is independently shippable and leaves the tool in a usable state.
Stop at any stage where the remaining features aren't pulling their weight.

### Stage 0 — Skeleton

**Goal**: workspace compiles, `kg --help` runs, CI-less local test loop works.

- `cargo new --lib crates/core`, `cargo new crates/cli`.
- Workspace `Cargo.toml`, shared `rust-version`, `edition = "2024"`.
- Dependencies pinned: `clap` (derive), `serde`, `serde_json`, `thiserror`,
  `anyhow` (binary only), `tracing` + `tracing-subscriber`.
- Error type in `kg-core`: one `Error` enum, `thiserror`-derived.
- CLI scaffold: `kg --help`, `kg --version`, global `--vault <path>`
  (fallback `$KG_VAULT_PATH`), global `--data-dir` (fallback
  `$KG_DATA_DIR`, default `$XDG_DATA_HOME/kg` or `~/.local/share/kg`).
- JSON output helper: every subcommand returns a `Serialize` struct; the CLI
  wraps it in `{"ok": true, "data": ...}` or `{"ok": false, "error": ...}`.

**Done when**: `cargo test` passes with zero tests; `kg parse --help` works
even though `parse` is a stub.

[x] This stage has been implemented.

---

### Stage 1 — Parser

**Goal**: turn a vault into a stream of nodes and edges. Useful on its own
(pipe into `jq`).

- Walk vault for `.md` files (`ignore` crate — respects `.gitignore`,
  handles symlinks, fast).
- Frontmatter: `gray_matter` crate (YAML). Tolerate malformed YAML — log
  and continue.
- Wiki links: hand-rolled parser. Match `[[target]]`, `[[target|alias]]`,
  `[[target#section]]`, `[[target#section|alias]]`. Skip matches inside
  fenced code blocks and inline code.
- ~~Inline tags: `#tag` not preceded by a word character, not inside code.~~ <!-- note: | We should intentionally ignore supporting Obsidian inline tags [i.e. prefix the word with `#`] by design. @2026-04-17 -->
- Edge context: the enclosing paragraph of each link (split on blank lines). <!-- note | we should cap the maximum sentences to `N` if the links are inside a really long paragraph. @2026-04-17 -->
- Output types (in `kg-core::types`): `ParsedNode { id, path, title, tags,
  frontmatter, first_paragraph }`, `ParsedEdge { source, target_raw,
  context }`. `target_raw` is the unresolved `[[...]]` text — resolution
  happens in Stage 2.

**CLI**: `kg parse` → streams one JSON object per node on stdout
(`{"type":"node",...}` / `{"type":"edge",...}`), or `kg parse --pretty` for
a single JSON document.

**Tests**: fixture vault under `crates/core/tests/fixtures/`. Cover malformed
YAML, code-fence escaping, tag detection, multi-paragraph files.

[x] This stage has been implemented.

---

### Stage 2 — Link resolver

**Goal**: resolve `[[target]]` → canonical node ID using Obsidian's
"shortest unique path" rules.

- Resolution order: exact path → basename-unique → full relative path.
- Ambiguous basename: pick first match, emit a warning to stderr.
- Name lookup helper (`resolve_name`) for later query-time use: id → exact
  title → case-insensitive title → alias (frontmatter `aliases`) → substring.
  Return `NameMatch { id, kind: Exact|CaseInsensitive|Alias|Substring,
  candidates: Vec<Id> }` so callers can distinguish ambiguous from missing.
- Unresolved link targets: Stage 3 decides whether to create stub nodes;
  the resolver just reports them.

**CLI**: `kg resolve <name>` — prints the `NameMatch` JSON. Handy for
debugging and composition.

**Tests**: port a few resolution cases from the TS suite.

[x] This stage has been implemented.

---

### Stage 3 — Store + indexer (no embeddings, no FTS yet)

**Goal**: persistent KG that you can rebuild incrementally.

- SQLite schema (clean slate, 6 tables):
  - `nodes(id TEXT PK, title TEXT, first_paragraph TEXT,
    frontmatter JSON, mtime INTEGER, is_stub INTEGER DEFAULT 0)`
    No separate `path` column — `id` already is the relative path.
  - `tags(node_id, tag)` + index on `node_id`
  - `aliases(node_id, alias)` + index on `node_id`
  - `edges(source, target, context)` + indexes on both `source` and `target`
  - `sync(path TEXT PK, mtime INTEGER)`
  - `meta(key TEXT PK, value TEXT)` — `schema_version = 1`
- Indexer diff: compare filesystem mtimes vs. `sync` table. Classify each
  file as NEW / CHANGED (fs mtime > stored) / UNCHANGED / DELETED.
  No-op shortcut: if added=0, changed=0, deleted=0, skip edge re-resolution.
  On non-trivial runs: full edge re-resolution (a new/deleted node can change
  resolution of links in unchanged files).
- Stub nodes: `id` = raw `target_raw` string (not a `.md` path), `is_stub=1`.
  Created via `INSERT OR IGNORE` so real nodes are never overwritten.
- mtime stays out of `ParsedNode` — the indexer collects mtimes separately
  during its filesystem walk.
- Default DB location: `<vault>/.kg/kg.db`, overridable via `--data-dir`.
- All SQL lives in `store.rs`. No SQL strings outside that module.
- WAL mode + `foreign_keys=ON` for all connections.

**CLI**:
- `kg index` — run the diff, print a summary (`{"added":N,"changed":N,
  "deleted":N,"stubs":N}`).
- `kg stats` — node/stub/edge/tag counts (tags = distinct values).

**Tests**: round-trip a fixture vault through index → re-index (no-op) →
touch a file → re-index (one changed) → delete a file → re-index (one
deleted). Full lifecycle integration test.

[x] This stage has been implemented.

---

### Stage 4 — Keyword search (FTS5)

**Goal**: `kg search "query"` returns matching nodes with excerpts.

- Schema migration v1 → v2: add denormalized `tags_text TEXT` column on
  `nodes`, create `nodes_fts` FTS5 virtual table (content-sync'd to
  `nodes` via `content=nodes, content_rowid=rowid`), three auto-sync
  triggers (INSERT/DELETE/UPDATE). Existing v1 databases upgrade
  transparently on next `Store::open` — `ALTER TABLE` adds the column,
  backfills from `tags` table, rebuilds FTS index. No forced re-index.
- FTS5 indexes three columns: `title`, `first_paragraph`, `tags_text`.
  `tags_text` is a space-joined denormalization of the `tags` table,
  populated in `upsert_node()`. Denormalization chosen over a subquery
  or separate FTS table because it keeps the trigger definitions simple
  and lets FTS5 weigh all three signals in a single `MATCH`.
- Stubs excluded from search results (`WHERE n.is_stub = 0`).
- BM25 ranking via `bm25(nodes_fts)` (negative values; sort ascending
  for best-first). Excerpts via `snippet(nodes_fts, -1, '[', ']',
  '...', 64)` — column index -1 lets FTS5 pick the best column; `[`/`]`
  as highlight markers (lightweight, no ANSI in JSON output); ~64 tokens.
- Result type: `SearchResult { id, title, score, excerpt }`.
- Default limit: 20.

**CLI**: `kg search <query> [--limit N]`. Output: bare NDJSON (one
`SearchResult` per line), matching the `parse`/`resolve` streaming style.

**Tests**: 20 new tests — `SearchResult` serialization, schema v2,
v1→v2 migration + backfill, `tags_text` population, FTS trigger
correctness (insert/delete/update), search by title/tag/paragraph,
BM25 ordering, stub exclusion, limit, no-matches, CLI smoke tests
(results/limit/no-matches/missing-vault).

[x] This stage has been implemented.

---

### Stage 5 — Graph queries

**Goal**: actually use the graph structure. No analytics yet — just
traversal.

- New module `graph.rs`: `KnowledgeGraph` struct wrapping
  `petgraph::DiGraph<String, ()>` + `HashMap<String, NodeIndex>` for
  O(1) ID lookup + `HashSet<String>` for stub tracking.
- Built once per CLI invocation via `KnowledgeGraph::from_store(&Store)`.
  No cross-process caching — the graph is rebuilt from SQLite each time
  (instant for vaults in the hundreds/low-thousands of nodes).
- New `Store` methods: `all_edges() → Vec<(String, String)>`,
  `all_nodes_metadata() → Vec<(String, bool)>`.
- New error variant: `NodeNotFound { id: String }` — returned by all four
  operations when a query references a node not in the graph.
- New types in `types.rs`: `NeighborEntry { id, depth }`,
  `SubgraphNode { id, is_stub }`, `SubgraphEdge { source, target }`,
  `Subgraph { nodes, edges }`.
- Operations:
  - `neighbors(id, depth, directed)` — BFS with `VecDeque`. Visited set
    initialized with the start node (excludes self-loops from results).
    Returns `Vec<NeighborEntry>` sorted by (depth, id).
  - `path(from, to, max_depth, directed)` — recursive DFS with visited
    set + backtracking. `max_depth` bounds edge count (path length - 1).
    Returns only simple paths (no repeated nodes), sorted
    lexicographically. Same-node queries return `[[id]]`.
  - `shared(a, b, directed)` — depth-1 neighbor set intersection.
    Excludes the two query nodes themselves. Returns sorted `Vec<String>`.
  - `subgraph(seeds, depth, directed)` — BFS from each seed up to depth,
    then filters all graph edges to those with both endpoints in the
    included set. Stubs marked `is_stub: true`. Nodes sorted by id,
    edges sorted by (source, target).
- Direction: all operations default to undirected (both incoming +
  outgoing edges); `--directed` restricts to outgoing only.

**CLI**: `kg neighbors <id> [--depth N] [--directed]`,
`kg path <from> <to> [--max-depth N] [--directed]`,
`kg shared <a> <b> [--directed]`,
`kg subgraph <id>... [--depth N] [--directed]`.
Output: single JSON object/array on stdout (not NDJSON), matching the
`stats`/`index` pattern since graph results are bounded and complete.

**Tests**: 52 new tests — 32 unit tests in `graph.rs` (construction,
neighbors, path, shared, subgraph with directed/undirected/edge cases),
8 integration tests against fixture vault in `graph_test.rs`,
4 Store method tests, 1 error serialization test, 7 CLI smoke tests.
Total: 210 tests.

[x] This stage has been implemented.

---

### Stage 6 — PageRank

**Goal**: one analytics signal that's actually useful for ranking.

- Remove degree-0 isolates, run power-iteration PageRank on the remaining graph (converted to undirected), return `(id, title, score)` sorted descending.
- Algorithm: damping=0.85, max_iter=100, epsilon=1e-6. Dangling nodes (out_degree=0 in undirected view) redistribute mass uniformly.
- Fallback to degree centrality if iteration fails to converge.
- Cache result in `meta` keyed by a graph fingerprint (`{node_count}:{edge_count}:{max_mtime}`) so `kg rank` is instant after `kg index`.
- New `Store` methods: `get_meta()`, `set_meta()`, `max_mtime()`, `node_titles()`, `graph_fingerprint()`.
- New type: `RankEntry { id, score }`.
- New `KnowledgeGraph` methods: `rank(top)`, `degree_centrality(top)`.

**CLI**: `kg rank [--top N]` — JSON array of `{id, title, score}`, default top 20.

**Tests**: 34 new tests — `RankEntry` serialization, store meta/mtime/fingerprint methods, PageRank unit tests (isolates, two-node, triangle, star, top limit, score sum, stubs, empty graph), degree centrality unit tests, integration tests against fixture vault, CLI smoke tests (valid output, --top, sorting, vault required, empty DB, cache consistency). Total: 244 tests.

[x] This stage has been implemented.

---

### Stage 7 — Embeddings + semantic search (shell-out)

**Goal**: optional semantic layer. Users plug in whatever embedder they like.

- Contract: `KG_EMBED_CMD` is a command that reads
  `{"texts":["...","..."]}` from stdin and writes
  `{"vectors":[[...],...],"dim":384}` to stdout. Batched.
- Store vectors as `BLOB` (f32 little-endian) on the `nodes` row.
  Embedder fingerprint (command + hash) lives in `meta`; if it changes,
  invalidate all vectors on next `kg index`.
- Embed input = `title + "\n" + tags.join(" ") + "\n" + first_paragraph`
  (same recipe as TS, easy to change later).
- KNN: brute-force cosine in Rust. Plenty fast for < ~50k nodes.
- Semantic search result merges with FTS via reciprocal rank fusion so
  `kg search --hybrid` can beat either alone.

**CLI**: `kg search <query> --semantic`, `kg search <query> --hybrid`.
`kg embed --dry-run` prints what would be sent to the embedder without
running it.

**Tests**: mock `KG_EMBED_CMD` with a fixture script that returns
deterministic vectors.

---

### Stage 8 (optional) — Louvain + betweenness

Defer until Stage 7 has proven the tool earns its keep. No good Rust Louvain
crate exists; plan to port the reference modularity-optimization algorithm
(~200 lines). Betweenness is available in `petgraph`.

Don't start this unless you've actually wanted communities in daily use.

---

## Milestone cadence

Stages 0–5 are the must-haves: they get you from "nothing" to "I can
parse my vault, incrementally index it, search it, and traverse the
graph." Stage 6 adds ranking. Stage 7 is the big optional feature.
Stage 8 is speculative.

Suggested order of attack: 0 → 1 → 2 → 3 → 4 → 5 → 6 → (evaluate) → 7.
