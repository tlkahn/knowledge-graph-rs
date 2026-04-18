use std::path::Path;

use gray_matter::Matter;
use gray_matter::engine::YAML;
use ignore::WalkBuilder;
use tracing::warn;

use crate::error::Error;
use crate::types::{ParsedEdge, ParsedNode, ParseEvent};
use crate::wiki_links::extract_wiki_links;

pub fn parse_file(vault_path: &Path, file_path: &Path) -> Result<(ParsedNode, Vec<ParsedEdge>), Error> {
    let content = std::fs::read_to_string(file_path).map_err(|e| Error::Io {
        source: e,
        path: file_path.to_path_buf(),
    })?;

    let id = file_path
        .strip_prefix(vault_path)
        .unwrap_or(file_path)
        .to_string_lossy()
        .to_string();

    let (frontmatter, body) = parse_file_content(&content);
    let tags = extract_tags(&frontmatter);
    let title = extract_title(&frontmatter, &id);
    let first_paragraph = extract_first_paragraph(&body);

    let node = ParsedNode {
        id: id.clone(),
        title,
        tags,
        frontmatter,
        first_paragraph,
    };

    let links = extract_wiki_links(&content);
    let edges: Vec<ParsedEdge> = links
        .into_iter()
        .map(|link| {
            let context = find_context(&link.target, &body);
            ParsedEdge {
                source: id.clone(),
                target_raw: link.target,
                context,
            }
        })
        .collect();

    Ok((node, edges))
}

pub fn parse_vault(vault_path: &Path) -> Result<Vec<ParseEvent>, Error> {
    if !vault_path.is_dir() {
        return Err(Error::VaultNotFound {
            path: vault_path.to_path_buf(),
        });
    }

    let mut events = Vec::new();
    let walker = WalkBuilder::new(vault_path).build();

    for entry in walker {
        let entry = entry.map_err(|e| Error::Io {
            source: std::io::Error::new(std::io::ErrorKind::Other, e.to_string()),
            path: vault_path.to_path_buf(),
        })?;

        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }

        let (node, edges) = parse_file(vault_path, path)?;
        events.push(ParseEvent::Node(node));
        for edge in edges {
            events.push(ParseEvent::Edge(edge));
        }
    }

    Ok(events)
}

fn parse_file_content(content: &str) -> (serde_json::Value, String) {
    let matter = Matter::<YAML>::new();

    let result: Result<gray_matter::ParsedEntity<serde_json::Value>, _> =
        matter.parse(content);
    match result {
        Ok(entity) => {
            let fm = entity
                .data
                .filter(|v| v.is_object())
                .unwrap_or_else(|| serde_json::json!({}));
            (fm, entity.content)
        }
        Err(e) => {
            warn!("malformed frontmatter: {e}");
            let body = strip_frontmatter_raw(content);
            (serde_json::json!({}), body)
        }
    }
}

fn strip_frontmatter_raw(content: &str) -> String {
    let mut lines = content.lines();
    match lines.next() {
        Some(line) if line.trim() == "---" => {}
        _ => return content.to_string(),
    }
    let mut found_close = false;
    let mut body_lines = Vec::new();
    for line in lines {
        if !found_close {
            if line.trim() == "---" {
                found_close = true;
            }
        } else {
            body_lines.push(line);
        }
    }
    if found_close {
        body_lines.join("\n")
    } else {
        content.to_string()
    }
}

fn extract_tags(fm: &serde_json::Value) -> Vec<String> {
    match fm.get("tags") {
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        Some(serde_json::Value::String(s)) => vec![s.clone()],
        _ => vec![],
    }
}

fn extract_title(fm: &serde_json::Value, id: &str) -> String {
    if let Some(serde_json::Value::String(t)) = fm.get("title") {
        return t.clone();
    }
    Path::new(id)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(id)
        .to_string()
}

fn extract_first_paragraph(body: &str) -> String {
    for para in body.split("\n\n") {
        let trimmed = para.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        return trimmed.to_string();
    }
    String::new()
}

fn find_context(target_raw: &str, body: &str) -> String {
    let needle = format!("[[{target_raw}");
    for para in body.split("\n\n") {
        if para.contains(&needle) {
            return para.trim().to_string();
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    // --- frontmatter parsing ---

    #[test]
    fn parses_valid_yaml_to_json() {
        let (fm, _) = parse_file_content("---\ntitle: Test\ntags:\n  - a\n  - b\n---\nbody");
        assert_eq!(fm["title"], "Test");
        assert_eq!(fm["tags"][0], "a");
        assert_eq!(fm["tags"][1], "b");
    }

    #[test]
    fn malformed_yaml_returns_empty_object() {
        let (fm, body) = parse_file_content("---\ntitle: [unclosed\n---\nbody here");
        assert!(fm.as_object().unwrap().is_empty());
        assert!(body.contains("body here"));
    }

    #[test]
    fn nested_yaml_becomes_nested_json() {
        let (fm, _) = parse_file_content("---\nouter:\n  inner: value\n---\n");
        assert_eq!(fm["outer"]["inner"], "value");
    }

    #[test]
    fn no_frontmatter_returns_empty_object() {
        let (fm, body) = parse_file_content("# Just a heading\n\nSome body text.");
        assert!(fm.as_object().unwrap().is_empty());
        assert!(body.contains("Just a heading"));
    }

    // --- tag extraction ---

    #[test]
    fn tags_array_of_strings() {
        let fm = serde_json::json!({"tags": ["a", "b"]});
        assert_eq!(extract_tags(&fm), vec!["a", "b"]);
    }

    #[test]
    fn tags_single_string() {
        let fm = serde_json::json!({"tags": "solo"});
        assert_eq!(extract_tags(&fm), vec!["solo"]);
    }

    #[test]
    fn no_tags_field() {
        assert!(extract_tags(&serde_json::json!({})).is_empty());
    }

    // --- title extraction ---

    #[test]
    fn title_from_frontmatter() {
        let fm = serde_json::json!({"title": "Alice"});
        assert_eq!(extract_title(&fm, "People/Alice Smith.md"), "Alice");
    }

    #[test]
    fn title_fallback_to_filename() {
        assert_eq!(
            extract_title(&serde_json::json!({}), "People/Alice Smith.md"),
            "Alice Smith"
        );
    }

    // --- first paragraph ---

    #[test]
    fn first_paragraph_skips_headings() {
        assert_eq!(extract_first_paragraph("# H\n\nBody\n\nMore"), "Body");
    }

    #[test]
    fn first_paragraph_no_headings() {
        assert_eq!(extract_first_paragraph("Body\n\nMore"), "Body");
    }

    #[test]
    fn only_heading_returns_empty() {
        assert_eq!(extract_first_paragraph("# H\n"), "");
    }

    #[test]
    fn first_paragraph_trimmed() {
        assert_eq!(
            extract_first_paragraph("# H\n\n  Body  \n\nMore"),
            "Body"
        );
    }

    // --- edge context ---

    #[test]
    fn finds_paragraph_containing_link() {
        let body = "First para.\n\nContains [[Widget Theory]] link.\n\nThird.";
        assert_eq!(
            find_context("Widget Theory", body),
            "Contains [[Widget Theory]] link."
        );
    }

    #[test]
    fn no_match_returns_empty() {
        assert_eq!(find_context("Missing", "No links here"), "");
    }

    #[test]
    fn context_is_trimmed() {
        let body = "First.\n\n  has [[X]] link  \n\nLast.";
        assert_eq!(find_context("X", body), "has [[X]] link");
    }

    // --- parse_file ---

    #[test]
    fn parse_file_returns_node_and_edges() {
        let vault = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vault");
        let file = vault.join("People/Alice Smith.md");
        let (node, edges) = parse_file(&vault, &file).unwrap();
        assert_eq!(node.id, "People/Alice Smith.md");
        assert_eq!(node.title, "Alice Smith");
        assert!(!edges.is_empty());
    }

    #[test]
    fn parse_file_error_for_nonexistent() {
        let vault = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vault");
        let file = vault.join("nonexistent.md");
        assert!(parse_file(&vault, &file).is_err());
    }
}
