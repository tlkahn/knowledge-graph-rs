use serde::{Serialize, Serializer, ser::SerializeStruct};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not implemented: {feature}")]
    NotImplemented { feature: String },
}

impl Error {
    fn kind(&self) -> &'static str {
        match self {
            Error::NotImplemented { .. } => "not_implemented",
        }
    }
}

impl Serialize for Error {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("Error", 2)?;
        s.serialize_field("kind", self.kind())?;
        s.serialize_field("message", &self.to_string())?;
        s.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_implemented_serializes_with_kind_and_message() {
        let err = Error::NotImplemented { feature: "parse".into() };
        let value = serde_json::to_value(&err).expect("serialize");
        assert_eq!(value["kind"], "not_implemented");
        let message = value["message"].as_str().expect("message is string");
        assert!(
            message.contains("parse"),
            "message should mention feature, got {message:?}",
        );
    }

    #[test]
    fn display_is_concise_human_string() {
        let err = Error::NotImplemented { feature: "parse".into() };
        assert_eq!(err.to_string(), "not implemented: parse");
    }
}
