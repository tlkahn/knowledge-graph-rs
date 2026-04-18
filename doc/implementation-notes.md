# Implementation Notes

Running log of design decisions, gotchas, and lessons learned during
the Rust re-implementation of the knowledge-graph tool.

---

## Stage 0 — Skeleton

Commit `b4568da`.

### JSON envelope protocol

Every CLI subcommand writes exactly one JSON document to stdout.
Success: `{"ok":true,"data":...}`. Error: `{"ok":false,"error":{"kind":"...","message":"..."}}`.
Tracing output goes to stderr exclusively so stdout remains machine-parseable.

Exit codes: 0 = success, 1 = runtime error (core `Error`), 2 = CLI usage error (clap).

The `Envelope<T>` type in `crates/cli/src/envelope.rs` owns this contract.
`kg_core::Error` implements `Serialize` with `kind` + `message` fields so
it drops directly into the error slot.

### Clap error routing

Clap prints its own error messages to stderr and calls `process::exit`.
We intercept via `Cli::try_parse()` and route errors ourselves:
help/version go to stdout (exit 0), everything else becomes an envelope
on stdout (exit 2). This keeps the "single JSON on stdout" invariant.

---

## Stage 1 — Parser

### gray_matter crate (0.3)

`serde_yaml` (dtolnay) is archived/unmaintained. We use `gray_matter`
which bundles `yaml-rust2` internally. Key API insight:

`Matter::<YAML>::parse()` is generic over the deserialization target.
Annotating the result type directly skips the intermediate `Pod` type:

```rust
let result: Result<ParsedEntity<serde_json::Value>, _> =
    matter.parse(content);
```

Without the type annotation the compiler cannot infer the generic
parameter and errors with E0282. The initial attempt used `Pod` with
`.and_then(|pod| pod.deserialize::<Value>())` which also hit this
inference failure. Specifying `ParsedEntity<serde_json::Value>` as
the result type is the cleanest fix — it deserializes YAML frontmatter
directly into `serde_json::Value`, no Pod conversion needed.

**Malformed YAML fallback**: when `gray_matter` returns `Err` (e.g.
`title: [unclosed`), we still need the body text. A `strip_frontmatter_raw`
helper manually scans for `---` delimiters and returns everything after
the closing one. This ensures the node still gets a `first_paragraph`
even when frontmatter parsing fails.

### ignore crate for vault walking

`WalkBuilder::new(path).build()` with default settings:
- Skips hidden files/dirs (`.obsidian/` excluded automatically)
- Respects `.gitignore` (correct behavior for real vaults)
- Handles symlinks safely

No need for manual directory filtering. We only filter on
`path.extension() == Some("md")` after the walker yields entries.

The `ignore` crate's walk errors are `ignore::Error`, not `std::io::Error`.
We convert via `std::io::Error::new(ErrorKind::Other, e.to_string())`
to fit our `Error::Io` variant.

### ParseEvent and serde tagging

`ParseEvent` uses `#[serde(tag = "type", rename_all = "snake_case")]`
(internally-tagged enum). This flattens variant fields into the object
with a `"type"` discriminator:

```json
{"type":"node","id":"...","title":"..."}
{"type":"edge","source":"...","target_raw":"..."}
```

This works because serde's internal tagging supports newtype variants
wrapping structs — the tag is merged into the inner struct's fields.

### Streaming vs. pretty output

The `parse` command has two output modes that break the envelope pattern:

- **Streaming (default)**: bare NDJSON lines, one per event. No envelope.
  This lets users pipe into `jq 'select(.type=="node")'` etc.
- **Pretty (`--pretty`)**: single JSON envelope `{"ok":true,"data":[...]}`,
  pretty-printed.

Errors always use the envelope regardless of mode.
The `dispatch()` function returns `Result<(), Error>` — `main` wraps
errors in `Envelope::err_from()`. For streaming success, we write
directly to `stdout.lock()` bypassing the envelope entirely.

### Wiki-link regex strategy

Three `LazyLock<Regex>` statics initialized once:
1. `FENCED_CODE` — `(?s)` ``` `` `.*?` `` ``` `|~~~.*?~~~` (dotall for multiline)
2. `INLINE_CODE` — `` `[^`]+` ``
3. `WIKI_LINK` — `\[\[([^\]]+)\]\]`

Processing order: strip code constructs first, then extract links from
the cleaned text. Embed detection checks `cleaned.as_bytes()[start - 1] == b'!'`
with a bounds guard.

Link inner text is parsed with split-on-first semantics:
`[[target#section|display]]` → split on first `|` for display, then
split left part on first `#` for section. Empty targets are filtered out.

### Edge context extraction

`find_context` searches the body (post-frontmatter) for paragraphs
containing `[[{target_raw}` as a substring. Paragraphs are delimited
by `\n\n`. This means edge context comes from the body, not from the
raw file content — frontmatter YAML lines won't accidentally match.

Note: links are extracted from the full file content (including frontmatter
region) via `extract_wiki_links(&content)`, but context is searched in
`body` only. Links that appear only in frontmatter YAML values (e.g.
`related: ["[[Foo]]"]`) will produce edges with empty context — this is
intentional and matches the TS behavior.

### first_paragraph extraction

Splits body on `\n\n`, skips paragraphs that are empty or start with `#`.
Returns the first paragraph that passes both filters, trimmed.

Heading-only files return `""`. Files where the first non-heading paragraph
is a list still capture it — bullet lists are not headings.

### Test structure

- **Unit tests**: inline `#[cfg(test)] mod tests` in each module. 40 tests
  covering pure functions (frontmatter parsing, tag/title extraction,
  paragraph extraction, wiki-link regex, code stripping).
- **Integration tests**: `crates/core/tests/parser_test.rs`. 13 tests
  running `parse_vault()` against the fixture vault. Tests node counts,
  field correctness, edge targets, code-fence filtering.
- **CLI smoke tests**: `crates/cli/tests/cli_smoke.rs`. 9 tests via
  `assert_cmd`. Cover NDJSON streaming, pretty envelope, missing vault,
  nonexistent vault, help output.

### id vs. path

Design decision from roadmap: nodes have `id` only (relative path from
vault root, e.g. `People/Alice Smith.md`). No separate `path: PathBuf`
field. Consumers construct `PathBuf` when needed via `vault_path.join(id)`.

### Inline tags intentionally excluded

The roadmap struck through inline `#tag` support. Tags come only from
frontmatter `tags` field (array or single string). This simplifies
parsing and avoids false positives in markdown headings.

---

## Stage 2 — Link Resolver

### The stem key idea

The central insight is that Obsidian wiki links like `[[Alice Smith]]`
omit both the directory prefix and the `.md` extension, while node IDs
are full relative paths (`People/Alice Smith.md`). We bridge this gap
with a "stem" key: the lowercased basename without `.md`.

`stem_of("People/Alice Smith.md")` → `"alice smith"`

This is a one-liner (`rsplit('/').next()`, strip `.md`, lowercase) but
it's the foundation everything else is built on. Getting this wrong
cascades everywhere — early testing of edge cases (no directory, nested
directories, no `.md` extension) paid off.

### StemLookup: three-tier resolution

`StemLookup` builds a `HashMap<String, Vec<String>>` keyed by stem plus
a `HashSet<String>` of all node IDs. Resolution follows three tiers:

1. **Exact-path match**: `target_raw + ".md"` exists in the ID set.
   This handles `[[People/Alice Smith]]` → `People/Alice Smith.md`
   directly. Also handles the edge case where users write `.md` in the
   link itself (`[[Widget Theory.md]]`).

2. **Stem match**: look up `stem_of(target_raw)` in the hash map.
   If there's exactly one candidate, it's unambiguous. This handles the
   common case: `[[Alice Smith]]` → unique `People/Alice Smith.md`.

3. **Path-suffix disambiguation**: when the stem has multiple candidates
   *and* the target contains `/`, check if any candidate's ID ends with
   `/{target_raw}.md` (case-insensitive). This handles
   `[[People/Alice Smith]]` disambiguating between `People/Alice Smith.md`
   and `Archive/Alice Smith.md`.

If still ambiguous: pick the first candidate alphabetically and emit
`tracing::warn`. This is deterministic (sorted candidates in each
bucket) and matches the TS implementation's "first match" behavior
while being reproducible across runs.

### resolve_edges: dedup strategy

After resolving each edge's `target_raw`, we dedup by `(source,
resolved_target)` keeping the first context encountered. This matches
Stage 1's whole-file dedup on raw targets but operates on resolved IDs
— two different `target_raw` values that resolve to the same node ID
get collapsed.

The dedup key for unresolved edges uses the raw `target_raw` string,
so two distinct broken links from the same source are kept separate.
Output is sorted by `(source, target_raw)` for determinism.

### resolve_name: 5-tier cascade

`resolve_name` is the query-time counterpart to `StemLookup::resolve`.
Where `resolve` turns wiki-link text into node IDs, `resolve_name`
finds nodes matching a user's search query. Five tiers, checked in
order — first tier with any hits wins:

1. **Id**: exact match on `node.id`
2. **Exact**: exact match on `node.title`
3. **CaseInsensitive**: lowercased title comparison
4. **Alias**: case-insensitive match against `frontmatter["aliases"]`
5. **Substring**: lowercased `query` contained in lowercased `title`

The early-return-on-first-hit design means an exact title match won't
also produce a substring result for the same node. This avoids confusing
output where a single node appears at multiple match tiers.

### Alias extraction

`extract_aliases` mirrors `parser.rs::extract_tags` — it handles both
`Value::Array` of strings and `Value::String` (single alias). This
consistency was intentional since several fixture files use different
alias formats: `aliases: ["A. Smith"]` vs `aliases: ["Widget Framework", "WT"]`.

### CLI output pattern for resolve

`kg resolve` follows the same bare-NDJSON pattern as `kg parse` (one
JSON object per line, no envelope wrapper). Empty output with exit 0
signals "no match" — this is not an error condition. This lets users
compose with `jq` and `wc -l` naturally:

```bash
kg resolve "Ali" --vault ~/vault | jq '.id'
kg resolve "Nothing" --vault ~/vault | wc -l   # → 0
```

### No new dependencies

Stage 2 is pure `HashMap` / `HashSet` / string operations + `tracing::warn`.
No new crate dependencies were needed — this kept the compile-time
impact minimal.

### Fixture vault changes

Added two files to the existing fixture vault:

- `Archive/Alice Smith.md` — creates an ambiguous basename (two files
  named `Alice Smith.md` in different directories). Title is
  `"Alice Smith (Archived)"` to differentiate.
- `Ambiguous.md` — exercises both ambiguous (`[[Alice Smith]]`) and
  unique (`[[Bob Jones]]`) links in a single document.

The parser integration test's node count assertion was updated from
9 → 11 to reflect the new files.

### Test structure for Stage 2

33 new tests across three locations:

- **Unit tests** in `resolve.rs` `#[cfg(test)]`: 30 tests organized by
  TDD cycle — `stem_of` (5), `StemLookup::build` (4),
  `StemLookup::resolve` (7), `resolve_edges` (5), `resolve_name` (9).
  Each cycle's tests use minimal synthetic data (test helpers `make_node`,
  `make_node_full`, `make_edge`) rather than the fixture vault, keeping
  them fast and focused.
- **CLI smoke tests** in `cli_smoke.rs`: 3 tests — successful resolve,
  empty result, missing vault error.

---

## Stage 3 — Store + Indexer

Commit: built on top of `d429ea0`.

### Schema design: dropping `path` from `nodes`

The original roadmap specified `nodes(id TEXT PK, path TEXT, ...)`. During
planning we realized `id` already is the relative path from vault root
(e.g. `People/Alice Smith.md`), established in Stage 1. Carrying a
redundant `path` column would create a synchronization hazard with no
benefit. Dropped it — consumers reconstruct `PathBuf` via
`vault_path.join(id)` when they need filesystem access.

### Extracting `parse_file` from `parse_vault`

The indexer needs to parse individual files (not the whole vault) for
changed/new entries. The inner loop of `parse_vault` (lines 33-65 at the
time) was extracted into a standalone `pub fn parse_file(vault_path, file_path)
-> Result<(ParsedNode, Vec<ParsedEdge>), Error>`. `parse_vault` now calls
`parse_file` internally, so behavior is identical and all 13 existing parser
integration tests continued passing without modification.

Key detail: `parse_file` reads from an absolute `file_path` but computes
`id` by stripping the `vault_path` prefix — the same `strip_prefix` logic
as before. The indexer passes `vault.join(relative_path)` as the
`file_path` argument.

### `extract_aliases` visibility change

`resolve::extract_aliases` was `fn` (private). `Store::upsert_node` needs
to extract aliases from frontmatter to populate the `aliases` table. Changed
to `pub(crate) fn` — visible within `kg-core` but not to external consumers.
This avoids duplicating the alias-extraction logic.

### rusqlite bundled feature

`rusqlite = { version = "0.35", features = ["bundled"] }` compiles SQLite
from C source as part of the build. This adds ~10s to a clean build but
eliminates the need for users to have `libsqlite3-dev` installed. Worth it
for a CLI tool.

### Schema migration strategy

`Store::migrate()` uses `CREATE TABLE IF NOT EXISTS` for all 6 tables and
`INSERT OR IGNORE INTO meta` for the schema version. This makes
`Store::open()` idempotent — opening an existing database is a no-op.

`PRAGMA journal_mode=WAL` is set on every open. This is safe to call
repeatedly (it's a no-op if already WAL). WAL mode avoids locking issues
if a future stage adds concurrent readers.

### The edge re-resolution tradeoff

The indexer re-resolves ALL edges on every non-trivial run (any add, change,
or delete). This is simpler than tracking which edges might be affected by
a node rename/add/delete:

- A new file can satisfy previously-unresolved `[[links]]`.
- A deleted file can make previously-resolved links ambiguous or unresolved.
- A renamed file affects both its own outgoing links and all incoming links.

The cost is re-parsing unchanged files for their edges. For the 11-file
fixture vault this is instant. For larger vaults (thousands of files), this
will be the first bottleneck to optimize — probably by caching parsed edges
in the store and only re-parsing the filesystem for changed files.

`replace_all_edges()` implements this with DELETE-all + batch INSERT rather
than diffing the old and new edge sets. Simpler and correct.

### Stub node semantics

Unresolved link targets get stub nodes with `is_stub=1`. The stub `id` is
the raw `target_raw` string (e.g. `"Nonexistent Page"`), not a `.md` path.
This is intentional — we don't know where the file would live if it existed.

`INSERT OR IGNORE` ensures that if a real file later appears with the same
name, `upsert_node` (which uses `INSERT OR REPLACE`) overwrites the stub
with full node data and `is_stub=0`. The stub is never accidentally
preserved over real data because `OR REPLACE` wins over `OR IGNORE`.

### mtime collection: separate from parsing

The plan considered adding `mtime` to `ParsedNode` but rejected it — mtime
is a filesystem concern, not a parse concern. Instead, `collect_vault_files`
walks the vault and returns `Vec<(String, i64)>` (relative path, epoch
seconds). The indexer passes the mtime to `store.upsert_node()` as a
separate argument.

This keeps `ParsedNode` purely about content. Tests that construct
`ParsedNode` values don't need to invent fake mtimes.

### Transaction boundaries

`index_vault` wraps the mutating phase (delete old nodes, upsert new/changed
nodes, replace all edges, create stubs) in a single transaction via
`BEGIN` / `COMMIT`. If any step fails, the partially-applied changes are
rolled back automatically when the connection drops (SQLite's implicit
rollback).

The no-op path (no changes detected) returns early before opening a
transaction — no unnecessary locking.

### CLI data-dir resolution

Default: `<vault>/.kg/`. The `--data-dir` flag (or `KG_DATA_DIR` env)
overrides it. `cmd_index` creates the directory (including parents) before
opening the database. `cmd_stats` does not create it — if the directory
doesn't exist, SQLite creates an empty database file (which returns all-zero
stats, the correct behavior for "never indexed").

### Refactoring `require_vault` in main.rs

Three subcommands (`parse`, `resolve`, `index`) had identical
vault-requirement boilerplate. Extracted into `require_vault(vault:
Option<PathBuf>) -> Result<PathBuf, Error>`. Similarly, `resolve_data_dir`
centralizes the `--data-dir` default logic. Small wins that remove
copy-paste without adding abstraction.

### Test patterns for mutable vault operations

Tests that touch/delete files cannot operate on the shared fixture vault
(that would break parallel test execution and leave the repo dirty). Pattern:
`copy_vault_to_tmp()` copies the entire fixture vault into a `tempfile::TempDir`,
returns the `TempDir` (which auto-cleans on drop). Tests then mutate files
in the temp copy.

The touch-detection test needs a filesystem mtime change, which requires
the write to happen at least 1 second after the initial index (filesystem
mtime granularity). A `std::thread::sleep(Duration::from_secs(1))` before
the rewrite ensures the mtime advances. This makes the test ~1s slower but
is the simplest reliable approach.

### Stats: distinct tags

`stats.tags` counts `COUNT(DISTINCT tag)` from the `tags` table — the number
of unique tag values across the vault, not the total number of tag
associations. For example, if 5 nodes all have tag `"person"`, that
contributes 1 to the tag count, not 5. This matches the TS implementation's
behavior.

### Test counts

Stage 3 added 41 new tests:
- **26 unit tests** in `store.rs` `#[cfg(test)]`: schema creation, CRUD
  operations, edge operations, stub semantics, sync queries, stats.
- **3 integration tests** in `store_test.rs`: open/idempotent/stats via
  public API.
- **7 integration tests** in `indexer_test.rs`: collect_vault_files,
  initial index, no-op re-index, touch detection, deletion detection,
  full lifecycle.
- **5 CLI smoke tests** in `cli_smoke.rs`: index output, index requires
  vault, re-index no-op, stats after index, stats on empty db.

Total across all stages: 138 tests (96 core lib + 7 indexer + 13 parser +
3 store + 2 envelope + 17 CLI).

---

## Stage 4 — FTS5 Keyword Search

Built on top of commit `647b130`.

### Schema versioning as a migration driver

Stage 3 planted a `meta(key, value)` table with `schema_version = '1'`.
Stage 4 leans on this for its v1 → v2 migration. The `migrate()` method
runs unconditionally on every `Store::open()`:

1. Create base v1 tables via `CREATE TABLE IF NOT EXISTS` (idempotent).
2. `INSERT OR IGNORE INTO meta` sets version to `'1'` for fresh DBs.
3. Read `schema_version`. If < 2: run the ALTER/backfill/version-bump
   sequence.
4. Create FTS5 table + triggers via `IF NOT EXISTS` (idempotent for
   both fresh-v2 and reopened-v2 databases).
5. If the migration just ran (version was < 2): `INSERT INTO
   nodes_fts(nodes_fts) VALUES ('rebuild')` to populate FTS from the
   backfilled `tags_text`.

This means fresh databases go through the v1 → v2 path too (the ALTER
TABLE on an empty `nodes` table is instant). One code path, no branching
between "fresh" and "upgrade" — the simplicity is worth the no-op ALTER.

### Why `tags_text` instead of a JOIN or separate FTS table

Three options were considered:

**(a) Denormalized `tags_text` column + single FTS table** — chosen.
Adding `tags_text TEXT DEFAULT ''` to `nodes` and including it in the
FTS5 column list keeps the trigger definitions trivial (three triggers,
each referencing `new.tags_text` / `old.tags_text`). The column is
populated in `upsert_node()` via `node.tags.join(" ")`.

**(b) FTS over a VIEW joining `nodes` and `tags`** — rejected. FTS5's
`content=` option requires a real table, not a view. You can fake it
with `content=''` (contentless) but then `snippet()` doesn't work (no
content to extract from). We need snippets.

**(c) Separate `tags_fts` table** — rejected. Querying across two FTS
tables requires `UNION` or application-level rank merging. BM25 scores
from different FTS tables aren't directly comparable. One table with all
three columns lets FTS5 do a single-pass `MATCH` with unified ranking.

### FTS5 content-sync mode

`content=nodes, content_rowid=rowid` tells FTS5 to read content from
the `nodes` table when running `snippet()` or `highlight()`. The FTS
index itself stores only tokens and positions, not the original text.
This avoids doubling storage for `title` and `first_paragraph`.

The trade-off: we must keep the FTS index manually synchronized with
the `nodes` table. Three triggers handle this:

- **AFTER INSERT**: straight insert into `nodes_fts`.
- **AFTER DELETE**: FTS5's special delete syntax (`INSERT INTO
  nodes_fts(nodes_fts, rowid, ...) VALUES ('delete', ...)`) removes
  the old entry by replaying the old tokens.
- **AFTER UPDATE**: delete-then-insert (same as above, two statements).

SQLite's `INSERT OR REPLACE` (used by `upsert_node`) is implemented as
DELETE + INSERT under the hood. So both the DELETE trigger and the INSERT
trigger fire, correctly updating the FTS index. For a new node (no
conflict), only the INSERT trigger fires. No special handling needed.

### `bm25()` returns negative scores

This surprised us initially. FTS5's `bm25()` returns values where more
negative means more relevant (it's actually the negative of the BM25
score). `ORDER BY bm25(nodes_fts)` ascending puts the best matches
first. The `SearchResult.score` field exposes these raw negative values
to consumers — wrapping or negating them would add confusion without
benefit, since the ordering semantic is what matters.

### Snippet highlight markers: `[` / `]`

The `snippet()` function takes configurable highlight markers. We chose
`[` and `]` because:

- JSON-safe (no escaping needed).
- Lightweight in terminal output.
- Easy for downstream consumers to regex-replace with ANSI codes or HTML
  if desired: `s/\[/\x1b[1m/g; s/\]/\x1b[0m/g`.

The alternative (ANSI escape codes directly) would clutter JSON output
and break consumers that parse the excerpt as plain text.

### `snippet()` column index -1

Passing `-1` as the column index tells FTS5 to automatically pick the
column with the best matching snippet. This is better than hardcoding
a column because a search for a tag name should excerpt from `tags_text`,
while a search for a phrase should excerpt from `first_paragraph`. The
automatic selection handles this without branching logic.

The `64` parameter is the maximum number of tokens in the snippet. This
produces excerpts of roughly 1-2 sentences — enough context to assess
relevance without overwhelming the output.

### Stub exclusion

Stubs have `is_stub=1`, empty title, empty `first_paragraph`, and empty
`tags_text`. They still get inserted into the FTS index (the trigger fires
on any INSERT into `nodes`), but they're excluded from search results via
`WHERE n.is_stub = 0` in the JOIN. This is simpler than conditionally
suppressing the trigger — stubs have no searchable content anyway, so
even without the filter they'd rarely match a real query.

### `upsert_stub` and the `tags_text` DEFAULT

`upsert_stub` doesn't specify `tags_text` in its INSERT statement.
SQLite fills in the `DEFAULT ''` before the trigger fires. The FTS insert
trigger sees `new.tags_text = ''` and indexes an empty string, which is
correct — stubs have no tags. No code change was needed for stubs.

### The v1 migration backfill

For existing v1 databases with data:

```sql
UPDATE nodes SET tags_text = COALESCE(
    (SELECT group_concat(tag, ' ') FROM tags WHERE tags.node_id = nodes.id),
    ''
);
```

The `COALESCE(..., '')` handles nodes with no tags (the subquery returns
NULL when `group_concat` has no rows). After the backfill, `INSERT INTO
nodes_fts(nodes_fts) VALUES ('rebuild')` tells FTS5 to re-read all rows
from the `nodes` table and rebuild its index from scratch.

This is a one-time cost. After migration, the triggers keep FTS in sync
incrementally.

### `rusqlite` bundled includes FTS5

The `rusqlite = { features = ["bundled"] }` feature compiles SQLite from
C source. The `libsqlite3-sys` build script enables
`-DSQLITE_ENABLE_FTS5` by default when bundling. No additional feature
flag was needed — this was confirmed empirically (the FTS5 `CREATE
VIRTUAL TABLE` succeeded on the first try).

### Test patterns for schema migration

The v1 → v2 migration test manually constructs a v1 database by opening
a raw `rusqlite::Connection`, creating the v1 schema, inserting data,
then closing it. It then opens the same file via `Store::open()` and
asserts:

- `schema_version() == 2`
- `tags_text` is backfilled (non-empty for nodes that had tags)
- FTS search returns results for the pre-existing data

This pattern is portable to future v2 → v3 migrations: create a vN
database manually, open it through `Store`, verify the upgrade.

### CLI output pattern

`kg search` follows the bare-NDJSON pattern established by `parse` and
`resolve`. One `SearchResult` JSON object per line, no envelope. Empty
output with exit 0 means "no matches" — not an error.

```bash
kg search "Alice" --vault ~/vault          # NDJSON results
kg search "Alice" --vault ~/vault | wc -l  # count matches
kg search "Alice" --limit 1 --vault ~/vault | jq '.excerpt'
```

### Test counts

Stage 4 added 20 new tests:

- **2 unit tests** in `types.rs`: `SearchResult` serialization and
  round-trip.
- **14 unit tests** in `store.rs`: schema v2, FTS table existence,
  `tags_text` population, FTS trigger correctness (insert/delete/update),
  search by title/tag/paragraph, BM25 ordering, stub exclusion, limit,
  no-matches, v1→v2 migration with backfill.
- **1 integration test** in `store_test.rs`: search on empty DB.
- **4 CLI smoke tests** in `cli_smoke.rs`: search returns results,
  limit works, no-match returns empty, missing vault errors.

Total across all stages: 158 tests (111 core lib + 7 indexer + 13 parser
+ 4 store + 2 envelope + 21 CLI).
