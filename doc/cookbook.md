# Cookbook

Practical recipes for the `kg` CLI. All examples assume you have an Obsidian vault at `~/vault`. Substitute your own path, or set the environment variable once:

```bash
export KG_VAULT_PATH=~/vault
```

## Setup: index your vault

Every query command (except `parse` and `resolve`) requires an indexed database. Run this first, and again whenever your vault changes:

```bash
kg index --vault ~/vault
```

Output:

```json
{"nodes_upserted":42,"nodes_deleted":0,"edges_replaced":118}
```

Re-running is incremental — only changed files are re-parsed. Deleted files are cleaned up automatically.

### Custom database location

By default the database lives at `<vault>/.kg/kg.db`. Override with `--data-dir`:

```bash
kg index --vault ~/vault --data-dir ~/.kg-data
kg stats --vault ~/vault --data-dir ~/.kg-data
```

## parse — raw vault inspection

Stream every node and edge as NDJSON, one JSON object per line. No database needed.

```bash
kg parse --vault ~/vault
```

```jsonl
{"type":"node","id":"People/Alice Smith.md","title":"Alice Smith","tags":["person","engineer"],...}
{"type":"edge","source":"People/Alice Smith.md","target_raw":"Widget Theory","context":"Lead engineer on the [[Widget Theory]] project"}
...
```

### Pretty envelope output

Wrap everything in a single `{"ok": true, "data": [...]}` envelope:

```bash
kg parse --vault ~/vault --pretty
```

### Count nodes and edges without indexing

```bash
kg parse --vault ~/vault | grep -c '"type":"node"'
kg parse --vault ~/vault | grep -c '"type":"edge"'
```

### Extract all tags used across the vault

```bash
kg parse --vault ~/vault \
  | jq -r 'select(.type == "node") | .tags[]' \
  | sort | uniq -c | sort -rn
```

### List all outgoing links from a specific note

```bash
kg parse --vault ~/vault \
  | jq -r 'select(.type == "edge" and .source == "People/Alice Smith.md") | .target_raw'
```

## resolve — fuzzy name lookup

Find nodes by ID, title, alias, or substring. No database needed — works directly from vault files. Streams matching nodes as NDJSON.

```bash
kg resolve "Alice Smith" --vault ~/vault
```

```json
{"id":"People/Alice Smith.md","title":"Alice Smith","tags":["person","engineer"],...}
```

### Case-insensitive and partial matches

Resolution checks five tiers in order: exact ID, exact title, case-insensitive title, alias, then substring. It returns all matches at the first tier that hits.

```bash
# Matches via alias "A. Smith"
kg resolve "A. Smith" --vault ~/vault

# Substring match — finds "Widget Theory" and anything else containing "widget"
kg resolve "widget" --vault ~/vault
```

### Check whether a note exists before scripting

```bash
if kg resolve "My Note" --vault ~/vault | head -1 | jq -e '.id' > /dev/null 2>&1; then
  echo "found"
else
  echo "not found"
fi
```

## stats — vault overview

Quick summary of the indexed graph:

```bash
kg stats --vault ~/vault
```

```json
{"nodes":42,"stubs":7,"edges":118,"tags":15}
```

`stubs` are nodes referenced by `[[wikilinks]]` but without a corresponding `.md` file.

## search — full-text search

FTS5-powered keyword search with BM25 ranking and snippet excerpts. Results stream as NDJSON.

```bash
kg search "distributed systems" --vault ~/vault
```

```json
{"id":"People/Alice Smith.md","title":"Alice Smith","score":-3.21,"excerpt":"...emergent patterns in [distributed] [systems]..."}
```

Matching terms are wrapped in `[brackets]` in the excerpt.

### Limit results

```bash
kg search "framework" --limit 5 --vault ~/vault
```

### FTS5 query syntax

The query supports SQLite FTS5 syntax:

```bash
# Phrase search
kg search '"component interactions"' --vault ~/vault

# Boolean OR
kg search "widget OR gadget" --vault ~/vault

# Prefix match
kg search "engin*" --vault ~/vault

# Exclude term
kg search "framework NOT gadget" --vault ~/vault
```

### Pipe search results into further queries

Find neighbors of the top search hit:

```bash
top_id=$(kg search "theory" --limit 1 --vault ~/vault | jq -r '.id')
kg neighbors "$top_id" --vault ~/vault
```

## neighbors — BFS exploration

Find nodes connected to a given node. Default depth is 1 (immediate neighbors).

```bash
kg neighbors "People/Alice Smith.md" --vault ~/vault
```

```json
[{"id":"Concepts/Widget Theory.md","depth":1},{"id":"Ideas/Acme Project.md","depth":1},{"id":"People/Bob Jones.md","depth":1}]
```

### Deeper traversal

Expand to 2 hops:

```bash
kg neighbors "People/Alice Smith.md" --depth 2 --vault ~/vault
```

### Directed neighbors (outgoing links only)

```bash
kg neighbors "People/Alice Smith.md" --directed --vault ~/vault
```

This only follows links *from* Alice's note, ignoring notes that link *to* her.

### List neighbor IDs for scripting

```bash
kg neighbors "People/Alice Smith.md" --vault ~/vault | jq -r '.[].id'
```

### Find isolated notes (no neighbors)

Cross-reference with `parse` to find orphans:

```bash
kg parse --vault ~/vault \
  | jq -r 'select(.type == "node") | .id' \
  | while read -r id; do
      count=$(kg neighbors "$id" --vault ~/vault | jq 'length')
      [ "$count" -eq 0 ] && echo "$id"
    done
```

## path — connection discovery

Find all simple paths between two nodes. Great for discovering how concepts relate.

```bash
kg path "People/Alice Smith.md" "Concepts/Gadget Pattern.md" --vault ~/vault
```

```json
[["People/Alice Smith.md","Concepts/Widget Theory.md","Concepts/Gadget Pattern.md"],["People/Alice Smith.md","Ideas/Acme Project.md","Concepts/Gadget Pattern.md"]]
```

### Limit path length

Only find short connections (at most 3 edges):

```bash
kg path "People/Alice Smith.md" "Concepts/Gadget Pattern.md" --max-depth 3 --vault ~/vault
```

### Directed paths only

Follow link direction strictly — useful when you care about *authorship flow* (who links to whom):

```bash
kg path "People/Alice Smith.md" "Concepts/Gadget Pattern.md" --directed --vault ~/vault
```

### Check if two nodes are connected

```bash
paths=$(kg path "orphan.md" "People/Alice Smith.md" --vault ~/vault)
if [ "$paths" = "[]" ]; then
  echo "no connection"
else
  echo "connected"
fi
```

## shared — find common ground

Discover nodes that two notes both link to (or are linked from). Useful for finding thematic overlap.

```bash
kg shared "People/Alice Smith.md" "People/Bob Jones.md" --vault ~/vault
```

```json
["Concepts/Widget Theory.md","Ideas/Acme Project.md"]
```

### Directed shared neighbors

Only count outgoing links:

```bash
kg shared "People/Alice Smith.md" "People/Bob Jones.md" --directed --vault ~/vault
```

## subgraph — focused extraction

Extract a subgraph around one or more seed nodes. Returns both nodes and the edges between them.

```bash
kg subgraph "People/Alice Smith.md" "People/Bob Jones.md" --vault ~/vault
```

```json
{"nodes":[{"id":"People/Alice Smith.md","is_stub":false},{"id":"People/Bob Jones.md","is_stub":false},{"id":"Concepts/Widget Theory.md","is_stub":false},...], "edges":[{"source":"People/Alice Smith.md","target":"Concepts/Widget Theory.md"},...]}
```

### Wider expansion

Grow the subgraph 2 hops out from each seed:

```bash
kg subgraph "Concepts/Widget Theory.md" --depth 2 --vault ~/vault
```

### Export to Graphviz DOT

Convert the subgraph JSON to a visual graph:

```bash
kg subgraph "People/Alice Smith.md" --depth 1 --vault ~/vault \
  | jq -r '
    "digraph {",
    "  rankdir=LR;",
    "  node [shape=box];",
    (.edges[] | "  \"\(.source)\" -> \"\(.target)\";"),
    "}"
  ' \
  | dot -Tpng -o subgraph.png
```

### Export to Mermaid

```bash
kg subgraph "People/Alice Smith.md" --depth 1 --vault ~/vault \
  | jq -r '
    "graph LR",
    (.edges[] | "  \(.source | gsub("[^a-zA-Z0-9]";"_"))[\"\(.source)\"] --> \(.target | gsub("[^a-zA-Z0-9]";"_"))[\"\(.target)\"]")
  '
```

### Count edges in a subgraph

```bash
kg subgraph "Concepts/Widget Theory.md" --vault ~/vault | jq '.edges | length'
```

## rank — find important nodes

PageRank centrality analysis. Identifies the most connected and referenced nodes.

```bash
kg rank --vault ~/vault
```

```json
[{"id":"Concepts/Widget Theory.md","title":"Widget Theory","score":0.187},{"id":"People/Alice Smith.md","title":"Alice Smith","score":0.154},...]
```

### Top N

```bash
kg rank --top 5 --vault ~/vault
```

### Get the single most important node

```bash
kg rank --top 1 --vault ~/vault | jq -r '.[0].title'
```

### Rank as a TSV table

```bash
kg rank --top 20 --vault ~/vault \
  | jq -r '.[] | [.score, .title] | @tsv' \
  | column -t
```

## Composing commands

### "What should I write about next?"

Find stub nodes (referenced but don't exist yet), ranked by how many notes link to them:

```bash
kg parse --vault ~/vault \
  | jq -r 'select(.type == "edge") | .target_raw' \
  | sort | uniq -c | sort -rn | head -10
```

Cross-reference with `stats` to see how many stubs your vault has:

```bash
kg stats --vault ~/vault | jq '.stubs'
```

### "Map a topic cluster"

Start from a concept, expand its neighborhood, then extract a visual subgraph:

```bash
# See what's around "Widget Theory"
kg neighbors "Concepts/Widget Theory.md" --depth 2 --vault ~/vault

# Extract it as a subgraph
kg subgraph "Concepts/Widget Theory.md" --depth 2 --vault ~/vault > cluster.json
```

### "How are these two areas connected?"

```bash
# Direct connections
kg shared "People/Alice Smith.md" "Concepts/Gadget Pattern.md" --vault ~/vault

# All paths (if shared returns empty, there may still be longer paths)
kg path "People/Alice Smith.md" "Concepts/Gadget Pattern.md" --max-depth 4 --vault ~/vault
```

### "Weekly vault health check"

```bash
echo "=== Stats ==="
kg stats --vault ~/vault | jq .

echo "=== Top 10 by PageRank ==="
kg rank --top 10 --vault ~/vault | jq -r '.[] | "\(.score | tostring | .[0:5])\t\(.title)"'

echo "=== Stubs (missing pages) ==="
kg stats --vault ~/vault | jq '.stubs'
```

## Error handling

All errors are returned as JSON on stdout with exit code 1:

```json
{"ok":false,"error":{"kind":"vault_not_found","message":"..."}}
```

Common errors:

| Scenario | Kind | Fix |
|----------|------|-----|
| Missing `--vault` | `vault_not_found` | Pass `--vault` or set `KG_VAULT_PATH` |
| Node ID not in graph | `node_not_found` | Check the ID with `resolve` first |
| DB not indexed yet | `database` | Run `kg index` first |
| Invalid subcommand | `unknown_subcommand` | Run `kg --help` |

## Tips

- **Node IDs are relative paths**: `People/Alice Smith.md`, not absolute paths or bare titles.
- **`parse` and `resolve` don't need indexing** — they work directly from vault files.
- **Everything else needs `kg index` first**: `stats`, `search`, `neighbors`, `path`, `shared`, `subgraph`, `rank`.
- **`rank` caches results** in the database. Re-running is instant unless the graph changed.
- **Logs go to stderr**: set `RUST_LOG=debug` for verbose tracing without polluting JSON output.
- **All JSON on stdout**: safe to pipe into `jq`, redirect to files, or consume from scripts.
