//! Provider profile types.
//!
//! There is no type here that can carry a stored key out of the program. A
//! profile is described by [`ProviderInfo`], which has `has_key` and the last
//! four characters and nothing else. The only type that holds a key is
//! [`SecretText`], and it only goes in: it can be read from a request and has no
//! `Serialize`, no `Display` and a `Debug` that prints nothing.

use std::fmt;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::mirror::api_enum;

api_enum! {
    /// The wire protocol of a provider.
    ProviderProtocol <=> llm_client::Protocol {
        OpenAiChat => "openai_chat",
        AnthropicMessages => "anthropic_messages",
    }
}

api_enum! {
    /// Where a profile is read from. `Env` profiles are read-only.
    ProviderSource <=> llm_client::ProfileSource { Env => "env", File => "file" }
}

/// A key as it arrives in a request.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct SecretText(String);

impl SecretText {
    /// The raw text. Do not log or format it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretText(<redacted>)")
    }
}

/// What the last connection test found out about a provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProviderCapabilities {
    pub probe_version: u32,
    pub auth_ok: bool,
    pub stream_ok: bool,
    /// Milliseconds to the first streamed token.
    pub ttft_ms: Option<u64>,
    pub tokens_per_second: Option<f64>,
    /// The structured-output ladder level that works, 1 (native schema) to 4
    /// (prompt only). `None` when no level worked.
    pub structured_level: Option<u8>,
    /// The contracts that produced valid output in the test.
    pub contracts_ok: Vec<String>,
    /// Rate limits the provider reported in its headers, if it did.
    pub rate_limit_rpm: Option<u32>,
    pub rate_limit_rpd: Option<u32>,
}

/// A provider profile as the UI may see it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProviderInfo {
    /// The id to use in `/api/providers/{id}`. A profile that is saved again
    /// with a changed address or model gets a new id.
    pub id: i64,
    pub name: String,
    pub protocol: ProviderProtocol,
    pub base_url: String,
    pub model: String,
    pub has_key: bool,
    /// The last four characters of the key, when the key is long enough to show
    /// them safely.
    pub key_last4: Option<String>,
    pub source: ProviderSource,
    pub is_active: bool,
    pub capabilities: Option<ProviderCapabilities>,
    /// RFC 3339 time of the last successful connection test.
    pub probed_at: Option<String>,
    /// RFC 3339 time the scorer qualification test passed.
    pub qualified_at: Option<String>,
}

/// `GET /api/providers`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProviderList {
    /// The `env` profile first when there is one, then the saved profiles.
    pub providers: Vec<ProviderInfo>,
    pub active_id: Option<i64>,
    /// Why a source of profiles could not be read, for example an `.env` that
    /// sets some of the four variables but not all. Never contains a key.
    pub problems: Vec<String>,
}

/// `POST /api/providers`: creates a saved profile or replaces the one with the
/// same name. Profiles are saved to `providers.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[ts(export)]
pub struct SaveProviderRequest {
    pub name: String,
    pub protocol: ProviderProtocol,
    pub base_url: String,
    pub model: String,
    /// The key. When absent and a profile of this name exists, its stored key is
    /// kept; when absent for a new profile, the profile has no key (a local
    /// server may need none).
    #[serde(default)]
    #[ts(optional, as = "Option<String>")]
    pub api_key: Option<SecretText>,
    /// Remove the stored key. Ignored when `api_key` is given.
    #[serde(default)]
    #[ts(optional)]
    pub clear_key: Option<bool>,
    /// Make this the active profile. The first profile ever saved becomes active
    /// regardless.
    #[serde(default)]
    #[ts(optional)]
    pub make_active: Option<bool>,
}

/// How a connection test failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ProbeFailureKind {
    /// The test was stopped, for example because the program is closing.
    Cancelled,
    Timeout,
    /// The provider could not be reached.
    Network,
    /// The provider refused the key.
    Auth,
    RateLimited,
    /// The provider answered with a server error.
    ProviderError,
    /// The provider answered, but not in a way the client understands.
    UnexpectedReply,
    /// The address is not allowed: not HTTPS, or not the provider's own host.
    Configuration,
}

/// What went wrong in a connection test.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProbeFailure {
    pub kind: ProbeFailureKind,
    /// A sentence for the details line. Provider text in it has passed the
    /// key-removing filter of `llm-client`.
    pub message: String,
}

/// `POST /api/providers/{id}/test`. A provider that cannot be reached is a
/// normal answer with `ok: false`, not an HTTP error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProbeReport {
    pub provider_id: i64,
    pub ok: bool,
    pub capabilities: Option<ProviderCapabilities>,
    pub failure: Option<ProbeFailure>,
    pub duration_ms: u64,
}
