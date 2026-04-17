use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParsedNode {
    pub id: String,
    pub title: String,
    pub tags: Vec<String>,
    pub frontmatter: serde_json::Value,
    pub first_paragraph: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParsedEdge {
    pub source: String,
    pub target_raw: String,
    pub context: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ParseEvent {
    Node(ParsedNode),
    Edge(ParsedEdge),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parsed_node_serializes_to_json() {
        let node = ParsedNode {
            id: "People/Alice Smith.md".into(),
            title: "Alice Smith".into(),
            tags: vec!["person".into(), "engineer".into()],
            frontmatter: json!({"title": "Alice Smith", "tags": ["person", "engineer"]}),
            first_paragraph: "Lead engineer on the Widget Theory project.".into(),
        };
        let value = serde_json::to_value(&node).expect("serialize");
        assert_eq!(value["id"], "People/Alice Smith.md");
        assert_eq!(value["title"], "Alice Smith");
        assert_eq!(value["tags"], json!(["person", "engineer"]));
        assert!(value["frontmatter"].is_object());
        assert_eq!(
            value["first_paragraph"],
            "Lead engineer on the Widget Theory project."
        );
    }

    #[test]
    fn parsed_node_empty_optionals() {
        let node = ParsedNode {
            id: "test.md".into(),
            title: "test".into(),
            tags: vec![],
            frontmatter: json!({}),
            first_paragraph: String::new(),
        };
        let value = serde_json::to_value(&node).expect("serialize");
        assert_eq!(value["tags"], json!([]));
        assert_eq!(value["frontmatter"], json!({}));
        assert_eq!(value["first_paragraph"], "");
    }

    #[test]
    fn parsed_edge_round_trips() {
        let edge = ParsedEdge {
            source: "People/Alice Smith.md".into(),
            target_raw: "Widget Theory".into(),
            context: "Lead engineer on the [[Widget Theory]] project".into(),
        };
        let json_str = serde_json::to_string(&edge).expect("serialize");
        let back: ParsedEdge = serde_json::from_str(&json_str).expect("deserialize");
        assert_eq!(back, edge);
    }

    #[test]
    fn parse_event_node_has_type_tag() {
        let event = ParseEvent::Node(ParsedNode {
            id: "test.md".into(),
            title: "test".into(),
            tags: vec![],
            frontmatter: json!({}),
            first_paragraph: String::new(),
        });
        let value = serde_json::to_value(&event).expect("serialize");
        assert_eq!(value["type"], "node");
        assert_eq!(value["id"], "test.md");
    }

    #[test]
    fn parse_event_edge_has_type_tag() {
        let event = ParseEvent::Edge(ParsedEdge {
            source: "a.md".into(),
            target_raw: "B".into(),
            context: "links to [[B]]".into(),
        });
        let value = serde_json::to_value(&event).expect("serialize");
        assert_eq!(value["type"], "edge");
        assert_eq!(value["source"], "a.md");
    }
}
