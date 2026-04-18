# knowledge-graph-rs

A fast, read-only knowledge graph tool for Obsidian vaults, written in Rust.

Parses markdown files with YAML frontmatter, extracts wiki-link relationships,
resolves links to canonical node IDs, and indexes the resulting graph into
SQLite for persistent, incremental querying.
Full-text search is built in via SQLite FTS5 with BM25 ranking and excerpt
extraction. Designed to be composed via pipes — every subcommand emits JSON
to stdout.

## Quick start

```bash
# Requires Rust 1.94+
cargo build --release

# Index a vault into a local SQLite database (incremental on re-runs)
kg index --vault ~/my-vault

# Show graph statistics
kg stats --vault ~/my-vault

# Parse a vault into nodes and edges (NDJSON, one object per line)
kg parse --vault ~/my-vault

# Pretty-print as a single JSON document
kg parse --vault ~/my-vault --pretty

# Resolve a name to matching nodes
kg resolve "Alice Smith" --vault ~/my-vault

# Full-text search with BM25 ranking
kg search "distributed systems" --vault ~/my-vault

# Limit results
kg search "engineer" --limit 5 --vault ~/my-vault

# Pipe to jq
kg parse --vault ~/my-vault | jq 'select(.type=="node") | .title'
kg resolve "WT" --vault ~/my-vault | jq '.id'
kg search "Alice" --vault ~/my-vault | jq '.excerpt'
```

The vault path can also be set via `KG_VAULT_PATH` environment variable.
The database directory defaults to `<vault>/.kg/` and can be overridden
with `--data-dir` or `KG_DATA_DIR`.

## Output format

**Streaming mode** (default) — one JSON object per line:

```jsonl
{"type":"node","id":"People/Alice.md","title":"Alice","tags":["person"],"frontmatter":{...},"first_paragraph":"..."}
{"type":"edge","source":"People/Alice.md","target_raw":"Widget Theory","context":"Lead engineer on the [[Widget Theory]] project"}
```

**Pretty mode** (`--pretty`) — single JSON envelope:

```json
{
  "ok": true,
  "data": [...]
}
```

**Resolve output** — one match per line:

```jsonl
{"id":"People/Alice Smith.md","title":"Alice Smith","kind":"exact"}
{"id":"Archive/Alice Smith.md","title":"Alice Smith (Archived)","kind":"substring"}
```

Match kinds: `id`, `exact`, `case_insensitive`, `alias`, `substring` (checked in priority order — only the highest-matching tier is returned).

**Index output** — summary of changes applied:

```json
{"added":42,"changed":0,"deleted":0,"stubs":3}
```

**Search output** — one result per line, ranked by relevance:

```jsonl
{"id":"People/Alice Smith.md","title":"Alice Smith","score":-1.53,"excerpt":"[Alice] Smith"}
{"id":"Concepts/Widget Theory.md","title":"Widget Theory","score":-0.76,"excerpt":"Originally proposed by [[[Alice] Smith]]..."}
```

Scores are BM25 values (more negative = more relevant). Excerpts highlight matches with `[brackets]`.

**Stats output** — current graph counts:

```json
{"nodes":42,"stubs":3,"edges":128,"tags":15}
```

Errors always return `{"ok":false,"error":{"kind":"...","message":"..."}}` with a non-zero exit code.

## What gets parsed

- All `.md` files in the vault (hidden directories like `.obsidian/` are skipped)
- YAML frontmatter via `gray_matter` (malformed YAML is tolerated — the node still appears with empty frontmatter)
- Wiki links: `[[target]]`, `[[target|alias]]`, `[[target#section]]`, `[[target#section|alias]]`
- Links inside fenced code blocks and inline code are ignored
- Image embeds (`![[image.png]]`) are ignored
- Edge context: the enclosing paragraph of each link
- Tags from frontmatter `tags` field only (inline `#tag` syntax is not extracted)

## Link resolution

Wiki links like `[[Alice Smith]]` are resolved to canonical node IDs (e.g. `People/Alice Smith.md`) using Obsidian's shortest-unique-path rules:

1. **Exact path** — `[[People/Alice Smith]]` matches `People/Alice Smith.md` directly
2. **Unique basename** — `[[Bob Jones]]` matches the only `Bob Jones.md` in the vault
3. **Path-suffix disambiguation** — `[[People/Alice Smith]]` disambiguates when `Archive/Alice Smith.md` also exists

Ambiguous links (multiple files with the same basename, no path qualifier) pick the first match alphabetically and emit a warning.

## Persistence and incremental indexing

`kg index` builds a SQLite database (default: `<vault>/.kg/kg.db`) containing the full knowledge graph. Subsequent runs are incremental: only files whose mtime has changed since the last index are re-parsed. Deleted files are cleaned up, and new files are added.

Because adding or removing a node can change how wiki-links resolve across the entire vault, edges are fully re-resolved on every non-trivial index run. Stub nodes (`is_stub=1`) are created for link targets that don't correspond to any file.

`kg stats` reports current counts without re-indexing.

## Milestones

| Stage | Status | Description |
|-------|--------|-------------|
| 0 — Skeleton | Done | Workspace, CLI envelope protocol, `parse` stub |
| 1 — Parser | Done | Vault walker, frontmatter, wiki-links, NDJSON streaming |
| 2 — Link resolver | Done | Resolve `[[target]]` to canonical node IDs, `kg resolve` subcommand |
| 3 — Store + indexer | Done | SQLite persistence, incremental mtime-based re-indexing, `kg index` + `kg stats` |
| 4 — Keyword search | Done | FTS5 full-text search with BM25 ranking, snippet excerpts, `kg search` (158 tests) |
| 5 — Graph queries | Planned | Neighbors, paths, shared connections, subgraph extraction |
| 6 — PageRank | Planned | Ranking on largest connected component |
| 7 — Embeddings | Planned | Semantic search via external embedder (`KG_EMBED_CMD`) |

## License

MIT
