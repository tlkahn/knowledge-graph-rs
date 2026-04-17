use std::sync::LazyLock;

use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawLink {
    pub target: String,
    pub display: Option<String>,
    pub section: Option<String>,
}

static FENCED_CODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)```.*?```|~~~.*?~~~").unwrap());

static INLINE_CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`[^`]+`").unwrap());

static WIKI_LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[\[([^\]]+)\]\]").unwrap());

pub fn strip_code_constructs(md: &str) -> String {
    let without_fenced = FENCED_CODE.replace_all(md, "");
    INLINE_CODE.replace_all(&without_fenced, "").into_owned()
}

pub fn extract_wiki_links(md: &str) -> Vec<RawLink> {
    let cleaned = strip_code_constructs(md);
    let mut links = Vec::new();

    for cap in WIKI_LINK.captures_iter(&cleaned) {
        let m = cap.get(0).unwrap();
        if m.start() > 0 && cleaned.as_bytes()[m.start() - 1] == b'!' {
            continue;
        }

        let inner = &cap[1];
        if inner.is_empty() {
            continue;
        }

        let link = parse_inner(inner);
        if !link.target.is_empty() {
            links.push(link);
        }
    }

    links
}

fn parse_inner(inner: &str) -> RawLink {
    let (left, display) = match inner.find('|') {
        Some(pos) => {
            let d = inner[pos + 1..].to_string();
            (&inner[..pos], if d.is_empty() { None } else { Some(d) })
        }
        None => (inner, None),
    };

    let (target, section) = match left.find('#') {
        Some(pos) => {
            let s = left[pos + 1..].to_string();
            (&left[..pos], if s.is_empty() { None } else { Some(s) })
        }
        None => (left, None),
    };

    RawLink {
        target: target.to_string(),
        display,
        section,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- strip_code_constructs ---

    #[test]
    fn strips_fenced_code_blocks() {
        let md = "before\n```\n[[fake]]\n```\nafter";
        let result = strip_code_constructs(md);
        assert!(!result.contains("[[fake]]"));
        assert!(result.contains("before"));
        assert!(result.contains("after"));
    }

    #[test]
    fn strips_tilde_fenced_blocks() {
        let md = "before\n~~~\n[[fake]]\n~~~\nafter";
        let result = strip_code_constructs(md);
        assert!(!result.contains("[[fake]]"));
    }

    #[test]
    fn strips_inline_code() {
        let md = "text `[[fake]]` more text";
        let result = strip_code_constructs(md);
        assert!(!result.contains("[[fake]]"));
        assert!(result.contains("text"));
        assert!(result.contains("more text"));
    }

    #[test]
    fn preserves_non_code_content() {
        let md = "[[real link]] and other text";
        let result = strip_code_constructs(md);
        assert_eq!(result, md);
    }

    // --- extract_wiki_links ---

    #[test]
    fn extracts_bare_link() {
        let links = extract_wiki_links("text [[Alice Smith]] more");
        assert_eq!(links.len(), 1);
        assert_eq!(
            links[0],
            RawLink {
                target: "Alice Smith".into(),
                display: None,
                section: None,
            }
        );
    }

    #[test]
    fn extracts_path_qualified() {
        let links = extract_wiki_links("[[Concepts/Widget Theory]]");
        assert_eq!(links[0].target, "Concepts/Widget Theory");
    }

    #[test]
    fn extracts_pipe_alias() {
        let links = extract_wiki_links("[[Widget Theory|WT]]");
        assert_eq!(
            links[0],
            RawLink {
                target: "Widget Theory".into(),
                display: Some("WT".into()),
                section: None,
            }
        );
    }

    #[test]
    fn extracts_section_link() {
        let links = extract_wiki_links("[[Widget Theory#Props]]");
        assert_eq!(
            links[0],
            RawLink {
                target: "Widget Theory".into(),
                display: None,
                section: Some("Props".into()),
            }
        );
    }

    #[test]
    fn extracts_section_with_alias() {
        let links = extract_wiki_links("[[Widget Theory#Props|properties]]");
        assert_eq!(
            links[0],
            RawLink {
                target: "Widget Theory".into(),
                display: Some("properties".into()),
                section: Some("Props".into()),
            }
        );
    }

    #[test]
    fn ignores_embeds() {
        let links = extract_wiki_links("![[photo.png]] and [[real]]");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "real");
    }

    #[test]
    fn ignores_links_in_code_fences() {
        let md = "```\n[[fake]]\n```\n[[real]]";
        let links = extract_wiki_links(md);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "real");
    }

    #[test]
    fn ignores_links_in_inline_code() {
        let md = "`[[fake]]` and [[real]]";
        let links = extract_wiki_links(md);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "real");
    }

    #[test]
    fn extracts_multiple_links_one_line() {
        let md = "[[A]] then [[B]] and [[C]]";
        let links = extract_wiki_links(md);
        assert_eq!(links.len(), 3);
        assert_eq!(links[0].target, "A");
        assert_eq!(links[1].target, "B");
        assert_eq!(links[2].target, "C");
    }

    #[test]
    fn empty_brackets_ignored() {
        let links = extract_wiki_links("[[]]");
        assert!(links.is_empty());
    }

    #[test]
    fn empty_target_after_pipe_ignored() {
        let links = extract_wiki_links("[[|alias]]");
        assert!(links.is_empty());
    }
}
