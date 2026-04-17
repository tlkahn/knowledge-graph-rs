use std::fmt;
use std::io::{self, Write};

use serde::Serialize;
use serde_json::json;

#[derive(Debug)]
pub enum Envelope<T: Serialize> {
    Ok(T),
    Err(serde_json::Value),
}

impl<T: Serialize> Envelope<T> {
    pub fn ok(data: T) -> Self {
        Envelope::Ok(data)
    }

    pub fn err_from(error: &kg_core::Error) -> Envelope<T> {
        Envelope::Err(serde_json::to_value(error).expect("kg_core::Error serializes"))
    }

    pub fn err(kind: &str, message: impl Into<String>) -> Envelope<T> {
        Envelope::Err(json!({ "kind": kind, "message": message.into() }))
    }

    fn to_value(&self) -> serde_json::Value {
        match self {
            Envelope::Ok(data) => json!({
                "ok": true,
                "data": serde_json::to_value(data).expect("data serializes"),
            }),
            Envelope::Err(error) => json!({
                "ok": false,
                "error": error,
            }),
        }
    }
}

impl<T: Serialize> fmt::Display for Envelope<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = serde_json::to_string(&self.to_value()).map_err(|_| fmt::Error)?;
        f.write_str(&s)
    }
}

pub fn emit_stdout<T: Serialize>(envelope: &Envelope<T>) {
    let mut out = io::stdout().lock();
    let _ = writeln!(out, "{envelope}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_envelope_renders_with_data() {
        let env = Envelope::ok(42);
        assert_eq!(env.to_string(), r#"{"data":42,"ok":true}"#);
    }

    #[test]
    fn err_from_kg_core_error_includes_kind_and_message() {
        let err = kg_core::Error::NotImplemented { feature: "parse".into() };
        let env: Envelope<()> = Envelope::err_from(&err);
        let s = env.to_string();
        assert!(s.contains(r#""ok":false"#), "got {s}");
        assert!(s.contains(r#""kind":"not_implemented""#), "got {s}");
    }
}
