use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::types::{ParsedEdge, ParsedNode};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum LinkResolution {
    Resolved { id: String },
    Ambiguous { picked: String, candidates: Vec<String> },
    Unresolved,
}

pub struct StemLookup {
    stems: HashMap<String, Vec<String>>,
    ids: HashSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedEdge {
    pub source: String,
    pub target_raw: String,
    pub context: String,
    pub resolution: LinkResolution,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchKind {
    Id,
    Exact,
    CaseInsensitive,
    Alias,
    Substring,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NameMatch {
    pub id: String,
    pub title: String,
    pub kind: MatchKind,
}

impl StemLookup {
    pub fn build(node_ids: &[&str]) -> Self {
        let mut stems: HashMap<String, Vec<String>> = HashMap::new();
        let mut ids = HashSet::new();
        for &id in node_ids {
            ids.insert(id.to_string());
            stems.entry(stem_of(id)).or_default().push(id.to_string());
        }
        for candidates in stems.values_mut() {
            candidates.sort();
        }
        Self { stems, ids }
    }

    pub fn resolve(&self, target_raw: &str) -> LinkResolution {
        // 1. Exact-path match: target_raw + ".md" in ID set
        let with_md = format!("{target_raw}.md");
        if self.ids.contains(&with_md) {
            return LinkResolution::Resolved { id: with_md };
        }
        // Also try target_raw as-is (user included .md)
        if self.ids.contains(target_raw) {
            return LinkResolution::Resolved { id: target_raw.to_string() };
        }

        // 2. Stem lookup
        let stem = stem_of(target_raw);
        let candidates = match self.stems.get(&stem) {
            Some(c) => c,
            None => return LinkResolution::Unresolved,
        };

        if candidates.len() == 1 {
            return LinkResolution::Resolved { id: candidates[0].clone() };
        }

        // 3. Path-suffix disambiguation when target contains '/'
        if target_raw.contains('/') {
            let suffix_md = format!("/{with_md}");
            let suffix_raw = format!("/{target_raw}");
            let suffix_lower = suffix_md.to_lowercase();
            let suffix_raw_lower = suffix_raw.to_lowercase();
            for c in candidates {
                let c_lower = c.to_lowercase();
                if c_lower.ends_with(&suffix_lower) || c_lower.ends_with(&suffix_raw_lower) {
                    return LinkResolution::Resolved { id: c.clone() };
                }
            }
        }

        // Ambiguous: pick first sorted
        tracing::warn!(
            target_raw,
            picked = &candidates[0],
            count = candidates.len(),
            "ambiguous link resolution"
        );
        LinkResolution::Ambiguous {
            picked: candidates[0].clone(),
            candidates: candidates.clone(),
        }
    }
}

pub fn resolve_edges(nodes: &[ParsedNode], edges: &[ParsedEdge]) -> Vec<ResolvedEdge> {
    let node_ids: Vec<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
    let lookup = StemLookup::build(&node_ids);

    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut result = Vec::new();

    for edge in edges {
        let resolution = lookup.resolve(&edge.target_raw);
        let resolved_target = match &resolution {
            LinkResolution::Resolved { id } => id.clone(),
            LinkResolution::Ambiguous { picked, .. } => picked.clone(),
            LinkResolution::Unresolved => edge.target_raw.clone(),
        };
        let key = (edge.source.clone(), resolved_target);
        if seen.insert(key) {
            result.push(ResolvedEdge {
                source: edge.source.clone(),
                target_raw: edge.target_raw.clone(),
                context: edge.context.clone(),
                resolution,
            });
        }
    }

    result.sort_by(|a, b| (&a.source, &a.target_raw).cmp(&(&b.source, &b.target_raw)));
    result
}

pub fn resolve_name(query: &str, nodes: &[ParsedNode]) -> Vec<NameMatch> {
    let query_lower = query.to_lowercase();

    // Tier 1: ID match
    let id_matches: Vec<_> = nodes
        .iter()
        .filter(|n| n.id == query)
        .map(|n| NameMatch { id: n.id.clone(), title: n.title.clone(), kind: MatchKind::Id })
        .collect();
    if !id_matches.is_empty() {
        return id_matches;
    }

    // Tier 2: Exact title match
    let exact: Vec<_> = nodes
        .iter()
        .filter(|n| n.title == query)
        .map(|n| NameMatch { id: n.id.clone(), title: n.title.clone(), kind: MatchKind::Exact })
        .collect();
    if !exact.is_empty() {
        return exact;
    }

    // Tier 3: Case-insensitive title match
    let ci: Vec<_> = nodes
        .iter()
        .filter(|n| n.title.to_lowercase() == query_lower)
        .map(|n| NameMatch { id: n.id.clone(), title: n.title.clone(), kind: MatchKind::CaseInsensitive })
        .collect();
    if !ci.is_empty() {
        return ci;
    }

    // Tier 4: Alias match (case-insensitive)
    let alias: Vec<_> = nodes
        .iter()
        .filter(|n| {
            extract_aliases(&n.frontmatter)
                .iter()
                .any(|a| a.to_lowercase() == query_lower)
        })
        .map(|n| NameMatch { id: n.id.clone(), title: n.title.clone(), kind: MatchKind::Alias })
        .collect();
    if !alias.is_empty() {
        return alias;
    }

    // Tier 5: Substring match on title
    nodes
        .iter()
        .filter(|n| n.title.to_lowercase().contains(&query_lower))
        .map(|n| NameMatch { id: n.id.clone(), title: n.title.clone(), kind: MatchKind::Substring })
        .collect()
}

pub(crate) fn extract_aliases(fm: &serde_json::Value) -> Vec<String> {
    match fm.get("aliases") {
        Some(serde_json::Value::Array(arr)) => {
            arr.iter().filter_map(|v| v.as_str().map(String::from)).collect()
        }
        Some(serde_json::Value::String(s)) => vec![s.clone()],
        _ => vec![],
    }
}

pub fn stem_of(id: &str) -> String {
    let basename = id.rsplit('/').next().unwrap_or(id);
    basename.strip_suffix(".md").unwrap_or(basename).to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Cycle 1: stem_of ---

    #[test]
    fn stem_of_strips_dir_and_extension() {
        assert_eq!(stem_of("People/Alice Smith.md"), "alice smith");
    }

    #[test]
    fn stem_of_concept_path() {
        assert_eq!(stem_of("Concepts/Widget Theory.md"), "widget theory");
    }

    #[test]
    fn stem_of_no_directory() {
        assert_eq!(stem_of("orphan.md"), "orphan");
    }

    #[test]
    fn stem_of_nested_directory() {
        assert_eq!(stem_of("Notes/Sub/Deep.md"), "deep");
    }

    #[test]
    fn stem_of_no_extension_no_dir() {
        assert_eq!(stem_of("Already"), "already");
    }

    // --- Cycle 2: StemLookup::build ---

    #[test]
    fn build_groups_by_stem() {
        let ids = ["Notes/Alice.md", "People/Bob.md", "Archive/Alice.md"];
        let lookup = StemLookup::build(&ids);
        assert_eq!(lookup.stems.get("alice").unwrap().len(), 2);
        assert_eq!(lookup.stems.get("bob").unwrap().len(), 1);
    }

    #[test]
    fn build_no_directory() {
        let ids = ["Foo.md"];
        let lookup = StemLookup::build(&ids);
        assert_eq!(lookup.stems.get("foo").unwrap(), &["Foo.md"]);
    }

    #[test]
    fn build_empty_input() {
        let ids: [&str; 0] = [];
        let lookup = StemLookup::build(&ids);
        assert!(lookup.stems.is_empty());
        assert!(lookup.ids.is_empty());
    }

    #[test]
    fn build_case_insensitive_key() {
        let ids = ["Notes/ABC.md", "Notes/abc.md"];
        let lookup = StemLookup::build(&ids);
        assert_eq!(lookup.stems.get("abc").unwrap().len(), 2);
    }

    // --- Cycle 3: StemLookup::resolve ---

    #[test]
    fn resolve_exact_path() {
        let ids = ["People/Alice Smith.md", "Archive/Alice Smith.md"];
        let lookup = StemLookup::build(&ids);
        assert_eq!(
            lookup.resolve("People/Alice Smith"),
            LinkResolution::Resolved { id: "People/Alice Smith.md".into() }
        );
    }

    #[test]
    fn resolve_unique_basename() {
        let ids = ["People/Bob Jones.md"];
        let lookup = StemLookup::build(&ids);
        assert_eq!(
            lookup.resolve("Bob Jones"),
            LinkResolution::Resolved { id: "People/Bob Jones.md".into() }
        );
    }

    #[test]
    fn resolve_ambiguous_basename() {
        let ids = ["Archive/Alice Smith.md", "People/Alice Smith.md"];
        let lookup = StemLookup::build(&ids);
        let res = lookup.resolve("Alice Smith");
        match res {
            LinkResolution::Ambiguous { picked, candidates } => {
                assert_eq!(picked, "Archive/Alice Smith.md");
                assert_eq!(candidates.len(), 2);
            }
            other => panic!("expected Ambiguous, got {other:?}"),
        }
    }

    #[test]
    fn resolve_path_suffix_disambiguates() {
        let ids = ["Archive/Alice Smith.md", "People/Alice Smith.md"];
        let lookup = StemLookup::build(&ids);
        assert_eq!(
            lookup.resolve("People/Alice Smith"),
            LinkResolution::Resolved { id: "People/Alice Smith.md".into() }
        );
    }

    #[test]
    fn resolve_nonexistent() {
        let ids = ["People/Bob Jones.md"];
        let lookup = StemLookup::build(&ids);
        assert_eq!(lookup.resolve("NonExistent"), LinkResolution::Unresolved);
    }

    #[test]
    fn resolve_case_insensitive_stem() {
        let ids = ["People/Alice Smith.md"];
        let lookup = StemLookup::build(&ids);
        assert_eq!(
            lookup.resolve("alice smith"),
            LinkResolution::Resolved { id: "People/Alice Smith.md".into() }
        );
    }

    #[test]
    fn resolve_target_with_md_extension() {
        let ids = ["Concepts/Widget Theory.md"];
        let lookup = StemLookup::build(&ids);
        assert_eq!(
            lookup.resolve("Widget Theory.md"),
            LinkResolution::Resolved { id: "Concepts/Widget Theory.md".into() }
        );
    }

    // --- Cycle 4: resolve_edges ---

    fn make_node(id: &str) -> ParsedNode {
        ParsedNode {
            id: id.into(),
            title: stem_of(id),
            tags: vec![],
            frontmatter: serde_json::json!({}),
            first_paragraph: String::new(),
        }
    }

    fn make_edge(source: &str, target_raw: &str, context: &str) -> ParsedEdge {
        ParsedEdge {
            source: source.into(),
            target_raw: target_raw.into(),
            context: context.into(),
        }
    }

    #[test]
    fn resolve_edges_resolves_by_basename() {
        let nodes = [make_node("A.md"), make_node("Dir/B.md")];
        let edges = [make_edge("A.md", "B", "links to [[B]]")];
        let resolved = resolve_edges(&nodes, &edges);
        assert_eq!(resolved.len(), 1);
        assert_eq!(
            resolved[0].resolution,
            LinkResolution::Resolved { id: "Dir/B.md".into() }
        );
    }

    #[test]
    fn resolve_edges_unresolved_target() {
        let nodes = [make_node("A.md")];
        let edges = [make_edge("A.md", "Ghost", "see [[Ghost]]")];
        let resolved = resolve_edges(&nodes, &edges);
        assert_eq!(resolved[0].resolution, LinkResolution::Unresolved);
    }

    #[test]
    fn resolve_edges_dedup_same_source_target() {
        let nodes = [make_node("A.md"), make_node("B.md")];
        let edges = [
            make_edge("A.md", "B", "first [[B]]"),
            make_edge("A.md", "B", "second [[B]]"),
        ];
        let resolved = resolve_edges(&nodes, &edges);
        let a_to_b: Vec<_> = resolved.iter().filter(|e| e.source == "A.md").collect();
        assert_eq!(a_to_b.len(), 1);
        assert_eq!(a_to_b[0].context, "first [[B]]");
    }

    #[test]
    fn resolve_edges_different_sources_kept() {
        let nodes = [make_node("A.md"), make_node("B.md"), make_node("C.md")];
        let edges = [
            make_edge("A.md", "C", "from A [[C]]"),
            make_edge("B.md", "C", "from B [[C]]"),
        ];
        let resolved = resolve_edges(&nodes, &edges);
        assert_eq!(resolved.len(), 2);
    }

    #[test]
    fn resolve_edges_sorted_deterministically() {
        let nodes = [make_node("B.md"), make_node("A.md"), make_node("C.md")];
        let edges = [
            make_edge("B.md", "C", ""),
            make_edge("A.md", "C", ""),
        ];
        let resolved = resolve_edges(&nodes, &edges);
        assert_eq!(resolved[0].source, "A.md");
        assert_eq!(resolved[1].source, "B.md");
    }

    // --- Cycle 5: resolve_name ---

    fn make_node_full(id: &str, title: &str, fm: serde_json::Value) -> ParsedNode {
        ParsedNode {
            id: id.into(),
            title: title.into(),
            tags: vec![],
            frontmatter: fm,
            first_paragraph: String::new(),
        }
    }

    #[test]
    fn resolve_name_by_id() {
        let nodes = [make_node_full("People/Alice Smith.md", "Alice Smith", serde_json::json!({}))];
        let matches = resolve_name("People/Alice Smith.md", &nodes);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].kind, MatchKind::Id);
    }

    #[test]
    fn resolve_name_exact_title() {
        let nodes = [make_node_full("People/Alice Smith.md", "Alice Smith", serde_json::json!({}))];
        let matches = resolve_name("Alice Smith", &nodes);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].kind, MatchKind::Exact);
    }

    #[test]
    fn resolve_name_case_insensitive_title() {
        let nodes = [make_node_full("People/Alice Smith.md", "Alice Smith", serde_json::json!({}))];
        let matches = resolve_name("alice smith", &nodes);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].kind, MatchKind::CaseInsensitive);
    }

    #[test]
    fn resolve_name_alias() {
        let nodes = [make_node_full(
            "People/Alice Smith.md",
            "Alice Smith",
            serde_json::json!({"aliases": ["Ali"]}),
        )];
        let matches = resolve_name("Ali", &nodes);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].kind, MatchKind::Alias);
    }

    #[test]
    fn resolve_name_alias_case_insensitive() {
        let nodes = [make_node_full(
            "People/Alice Smith.md",
            "Alice Smith",
            serde_json::json!({"aliases": ["Ali"]}),
        )];
        let matches = resolve_name("ali", &nodes);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].kind, MatchKind::Alias);
    }

    #[test]
    fn resolve_name_substring() {
        let nodes = [make_node_full("People/Alice.md", "Alice", serde_json::json!({}))];
        let matches = resolve_name("lic", &nodes);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].kind, MatchKind::Substring);
    }

    #[test]
    fn resolve_name_no_match() {
        let nodes = [make_node_full("People/Alice.md", "Alice", serde_json::json!({}))];
        let matches = resolve_name("zzz_nothing", &nodes);
        assert!(matches.is_empty());
    }

    #[test]
    fn resolve_name_multiple_exact_matches() {
        let nodes = [
            make_node_full("A/Alice.md", "Alice", serde_json::json!({})),
            make_node_full("B/Alice.md", "Alice", serde_json::json!({})),
        ];
        let matches = resolve_name("Alice", &nodes);
        assert_eq!(matches.len(), 2);
        assert!(matches.iter().all(|m| m.kind == MatchKind::Exact));
    }

    #[test]
    fn resolve_name_highest_tier_wins() {
        let nodes = [make_node_full("People/Alice Smith.md", "Alice Smith", serde_json::json!({}))];
        let matches = resolve_name("People/Alice Smith.md", &nodes);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].kind, MatchKind::Id);
    }
}
