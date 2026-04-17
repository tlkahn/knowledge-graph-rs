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
