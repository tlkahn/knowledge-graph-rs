# CLAUDE.md

## What this project is

Rust re-implementation of the `knowledge-graph` TypeScript tool (sibling
repo at `~/Desktop/knowledge-graph`). Reads an Obsidian vault and produces
a queryable knowledge graph. Read-only against the vault, no MCP server.

See `roadmap.md` for the full stage plan (0-8). See
`doc/implementation-notes.md` for detailed learnings and gotchas.

## Commands

```bash
cargo test                      # 97 tests (unit + integration + CLI smoke)
cargo run --bin kg -- --help    # CLI help
cargo run --bin kg -- parse --vault <path>           # stream NDJSON
cargo run --bin kg -- parse --vault <path> --pretty  # envelope JSON
cargo run --bin kg -- resolve "Alice Smith" --vault <path>  # name resolution
```

## Project layout

```
Cargo.toml                  # workspace: crates/core + crates/cli
crates/core/src/
  lib.rs                    # re-exports Error; declares parser, resolve, types, wiki_links
  error.rs                  # Error enum: NotImplemented, Io, VaultNotFound
  types.rs                  # ParsedNode, ParsedEdge, ParseEvent
  wiki_links.rs             # RawLink, extract_wiki_links(), strip_code_constructs()
  parser.rs                 # parse_vault() + frontmatter/paragraph helpers
  resolve.rs                # StemLookup, resolve_edges(), resolve_name() — link resolution
crates/cli/src/
  main.rs                   # entry: tracing init, clap parse, dispatch, exit codes
  cli.rs                    # Cli struct (clap derive): --vault, --data-dir, subcommands
  envelope.rs               # JSON envelope: {"ok":true,"data":...} / {"ok":false,"error":...}
crates/core/tests/
  fixtures/vault/           # 11 .md files + .obsidian/ + attachments/ (excluded by walker)
  parser_test.rs            # integration tests against fixture vault
crates/cli/tests/
  cli_smoke.rs              # CLI binary tests via assert_cmd
```

## Architecture

Business logic lives in `crates/core` (`kg-core`). The CLI crate is a
thin shell: parse args, call core, format output. Stages add modules to
core and subcommands to CLI in lock-step.

Pipeline (Stages 1-2):
1. `parser::parse_vault()` walks vault via `ignore` crate (skips hidden dirs)
2. Per file: `gray_matter` splits frontmatter from body, deserializes
   YAML directly into `serde_json::Value` — malformed YAML falls back
   to manual `---` delimiter stripping
3. `wiki_links::extract_wiki_links()` strips code constructs then regex-matches
   `[[target]]` / `[[target|alias]]` / `[[target#section|alias]]`
4. Returns `Vec<ParseEvent>` (tagged enum: Node or Edge)
5. `resolve::resolve_edges()` resolves `target_raw` → canonical node IDs via
   `StemLookup` (exact-path → basename-unique → path-suffix disambiguation)
6. `resolve::resolve_name()` provides query-time 5-tier name matching
   (id → exact → case-insensitive → alias → substring)

## Conventions

- **Rust 2024 edition**, toolchain 1.94, resolver v3.
- Errors: `kg_core::Error` enum with `thiserror`. Implements `Serialize`
  (`kind` + `message` fields). CLI wraps in `Envelope` for stdout.
- CLI output: always JSON on stdout, logs on stderr. Exit 0/1/2.
- `parse` and `resolve` stream bare NDJSON by default; `parse --pretty`
  wraps in an envelope.
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

## What's next (Stage 3)

Store + indexer: SQLite persistence via `better-sqlite3`, incremental
mtime-based re-indexing, stub node creation for unresolved targets,
`resolve_name` adapter over stored data.
