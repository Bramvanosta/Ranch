//! What a connection points at: the kind of backend that serves it, and
//! that backend's endpoint, a JSON object only the backend reads.
//!
//! The rest of Ranch carries an endpoint without looking inside: the
//! backend of its kind checks it ([`EndpointSchema`] says what it asks
//! for) and reads it into a type of its own. Secrets are never part of an
//! endpoint; they go to the secret store.

use serde_json::{Map, Value};

/// The kind of backend a connection is served by (`bullmq-redis`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BackendKind(String);

impl BackendKind {
    pub fn new(kind: impl Into<String>) -> Self {
        Self(kind.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for BackendKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where a connection's backend is reached, as that backend describes it:
/// a JSON object, stored as it is.
#[derive(Debug, Clone, PartialEq)]
pub struct Endpoint(Value);

impl Endpoint {
    pub fn new(value: Value) -> Self {
        Self(value)
    }

    /// An endpoint with nothing in it, as an unreadable stored one reads.
    pub fn empty() -> Self {
        Self(Value::Object(Map::new()))
    }

    pub fn as_json(&self) -> &Value {
        &self.0
    }

    pub fn into_json(self) -> Value {
        self.0
    }
}

/// What a backend's endpoint holds, field by field: what the connection
/// form asks for. A secret field is asked for but never stored in the
/// endpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct EndpointSchema {
    pub fields: Vec<EndpointField>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EndpointField {
    /// The field's key in the endpoint, or in the secrets for a secret.
    pub key: String,
    pub label: String,
    pub kind: FieldKind,
    pub required: bool,
    /// What a new connection starts with.
    pub default: Option<Value>,
}

/// How a field is entered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    Number,
    Boolean,
    /// Kept in the secret store, never in the endpoint.
    Secret,
    /// A PEM certificate, pasted or read from a file.
    Pem,
    /// The SSH tunnel group: bastion, user, authentication, jump host. Its
    /// password is a secret too.
    SshTunnel,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_endpoint_is_carried_as_it_is() {
        let json = serde_json::json!({ "host": "h", "nested": { "a": [1, 2] } });
        assert_eq!(Endpoint::new(json.clone()).into_json(), json);
        assert_eq!(Endpoint::empty().as_json(), &serde_json::json!({}));
    }
}
