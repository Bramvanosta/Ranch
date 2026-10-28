use chrono::{DateTime, Utc};

/// What time it is, for what Ranch stores with a timestamp.
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}
