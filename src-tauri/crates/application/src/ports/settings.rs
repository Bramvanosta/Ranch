use async_trait::async_trait;
use ranch_domain::errors::RanchError;

use super::sealer::Check;

/// A setting whose value decides what an agent may do. The store seals it
/// on every write and says, on every read through [`SettingsStore::safety`],
/// whether the seal holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafetySetting {
    AiEnabled,
    ApprovalMode,
    ReadonlyDefault,
    ShareMode,
    SharePatterns,
}

impl SafetySetting {
    pub const ALL: [SafetySetting; 5] = [
        Self::AiEnabled,
        Self::ApprovalMode,
        Self::ReadonlyDefault,
        Self::ShareMode,
        Self::SharePatterns,
    ];

    /// The `kv_settings` key it is stored under.
    pub const fn key(self) -> &'static str {
        match self {
            Self::AiEnabled => "ai_enabled",
            Self::ApprovalMode => "mcp_approval_mode",
            Self::ReadonlyDefault => "mcp_readonly_default",
            Self::ShareMode => "ai_share_mode",
            Self::SharePatterns => "ai_share_patterns",
        }
    }

    pub fn of_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.key() == key)
    }
}

/// A safety setting as stored, and what its seal says. The value comes
/// with the verdict: a reader decides what an unsealed value means, it
/// cannot get one without being told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sealed {
    pub value: Option<String>,
    pub check: Check,
}

/// The app's settings: flat string keys, opaque string values. A safety
/// setting is sealed inside the store, so no write path can forget to seal
/// it and no `safety` read can skip the check.
// `async_trait` marks the boxed future `must_use` on top of the `Result`.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait SettingsStore: Send + Sync {
    /// A setting as stored; `None` when unset.
    async fn get(&self, key: &str) -> Result<Option<String>, RanchError>;
    /// Write a setting (one row per key). A safety setting is sealed with
    /// it; a seal is not a setting and is refused.
    async fn set(&self, key: &str, value: &str) -> Result<(), RanchError>;
    /// Forget a setting. A safety setting's absence is sealed too.
    async fn remove(&self, key: &str) -> Result<(), RanchError>;
    /// A safety setting and what its seal says.
    async fn safety(&self, setting: SafetySetting) -> Result<Sealed, RanchError>;
    /// Seal every safety setting as it stands: the first run with a key.
    async fn seal_all(&self) -> Result<(), RanchError>;
    /// Write safety settings (`None` forgets one) and leave their seals as
    /// they are, in one transaction: the values set when the key of the
    /// seals was lost. A seal that no longer matches reads as the strictest
    /// value in `ranch-mcp`, and these are the strictest.
    async fn write_unsealed(
        &self,
        values: &[(SafetySetting, Option<&str>)],
    ) -> Result<(), RanchError>;
}
