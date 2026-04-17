# CLAUDE.md

## What this project is

Rust re-implementation of the `knowledge-graph` TypeScript tool (sibling
repo at `~/Desktop/knowledge-graph`). Reads an Obsidian vault and produces
a queryable knowledge graph. Read-only against the vault, no MCP server.

See `roadmap.md` for the full stage plan (0-8). See
`doc/implementation-notes.md` for detailed learnings and gotchas.

## Commands

```bash
cargo test                      # 64 tests (unit + integration + CLI smoke)
cargo run --bin kg -- --help    # CLI help
cargo run --bin kg -- parse --vault <path>           # stream NDJSON
cargo run --bin kg -- parse --vault <path> --pretty  # envelope JSON
```

## Project layout

```
Cargo.toml                  # workspace: crates/core + crates/cli
crates/core/src/
  lib.rs                    # re-exports Error; declares parser, types, wiki_links
  error.rs                  # Error enum: NotImplemented, Io, VaultNotFound
  types.rs                  # ParsedNode, ParsedEdge, ParseEvent
  wiki_links.rs             # RawLink, extract_wiki_links(), strip_code_constructs()
  parser.rs                 # parse_vault() + frontmatter/paragraph helpers
crates/cli/src/
  main.rs                   # entry: tracing init, clap parse, dispatch, exit codes
  cli.rs                    # Cli struct (clap derive): --vault, --data-dir, subcommands
  envelope.rs               # JSON envelope: {"ok":true,"data":...} / {"ok":false,"error":...}
crates/core/tests/
  fixtures/vault/           # 9 .md files + .obsidian/ + attachments/ (excluded by walker)
  parser_test.rs            # integration tests against fixture vault
crates/cli/tests/
  cli_smoke.rs              # CLI binary tests via assert_cmd
```

## Architecture

Business logic lives in `crates/core` (`kg-core`). The CLI crate is a
thin shell: parse args, call core, format output. Future stages add
modules to core and subcommands to CLI in lock-step.

Pipeline so far (Stage 1):
1. `parser::parse_vault()` walks vault via `ignore` crate (skips hidden dirs)
2. Per file: `gray_matter` splits frontmatter from body, deserializes
   YAML directly into `serde_json::Value` — malformed YAML falls back
   to manual `---` delimiter stripping
3. `wiki_links::extract_wiki_links()` strips code constructs then regex-matches
   `[[target]]` / `[[target|alias]]` / `[[target#section|alias]]`
4. Returns `Vec<ParseEvent>` (tagged enum: Node or Edge)

## Conventions

- **Rust 2024 edition**, toolchain 1.94, resolver v3.
- Errors: `kg_core::Error` enum with `thiserror`. Implements `Serialize`
  (`kind` + `message` fields). CLI wraps in `Envelope` for stdout.
- CLI output: always JSON on stdout, logs on stderr. Exit 0/1/2.
- `parse` subcommand streams bare NDJSON by default; `--pretty` wraps
  in an envelope.
- Tests: unit tests inline (`#[cfg(test)]`), integration tests in
  `crates/*/tests/`. Fixture vault at `crates/core/tests/fixtures/vault/`.
- `id` = relative path from vault root (e.g. `People/Alice Smith.md`).
  No separate `path` field.
- Inline `#tag` parsing intentionally excluded — tags come from frontmatter only.
- `target_raw` on edges is the unresolved `[[...]]` text. Resolution is Stage 2.

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
- `Envelope::ok()` is currently unused (parse bypasses it for streaming).
  Will be used again when future subcommands return structured data.
- The `ignore` walker respects `.gitignore` by default. Test fixtures
  have no `.gitignore` so this is transparent, but be aware if adding
  fixture files that match gitignore patterns.

## What's next (Stage 2)

Link resolver: turn `target_raw` into canonical node IDs using Obsidian's
shortest-unique-path rules. New subcommand `kg resolve <name>`.
