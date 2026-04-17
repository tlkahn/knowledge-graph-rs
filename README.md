# knowledge-graph-rs

A fast, read-only knowledge graph tool for Obsidian vaults, written in Rust.

Parses markdown files with YAML frontmatter, extracts wiki-link relationships,
and (in future stages) indexes, searches, and analyzes the resulting graph.
Designed to be composed via pipes — every subcommand emits JSON to stdout.

## Quick start

```bash
# Requires Rust 1.94+
cargo build --release

# Parse a vault into nodes and edges (NDJSON, one object per line)
kg parse --vault ~/my-vault

# Pretty-print as a single JSON document
kg parse --vault ~/my-vault --pretty

# Pipe to jq
kg parse --vault ~/my-vault | jq 'select(.type=="node") | .title'
```

The vault path can also be set via `KG_VAULT_PATH` environment variable.

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

Errors always return `{"ok":false,"error":{"kind":"...","message":"..."}}` with a non-zero exit code.

## What gets parsed

- All `.md` files in the vault (hidden directories like `.obsidian/` are skipped)
- YAML frontmatter via `gray_matter` (malformed YAML is tolerated — the node still appears with empty frontmatter)
- Wiki links: `[[target]]`, `[[target|alias]]`, `[[target#section]]`, `[[target#section|alias]]`
- Links inside fenced code blocks and inline code are ignored
- Image embeds (`![[image.png]]`) are ignored
- Edge context: the enclosing paragraph of each link
- Tags from frontmatter `tags` field only (inline `#tag` syntax is not extracted)

## Milestones

| Stage | Status | Description |
|-------|--------|-------------|
| 0 — Skeleton | Done | Workspace, CLI envelope protocol, `parse` stub |
| 1 — Parser | Done | Vault walker, frontmatter, wiki-links, NDJSON streaming (64 tests) |
| 2 — Link resolver | Planned | Resolve `[[target]]` to canonical node IDs (Obsidian shortest-unique-path rules) |
| 3 — Store + indexer | Planned | SQLite persistence, incremental mtime-based re-indexing |
| 4 — Keyword search | Planned | FTS5 full-text search with excerpts |
| 5 — Graph queries | Planned | Neighbors, paths, shared connections, subgraph extraction |
| 6 — PageRank | Planned | Ranking on largest connected component |
| 7 — Embeddings | Planned | Semantic search via external embedder (`KG_EMBED_CMD`) |

## License

MIT
