//! Provider profiles: which protocol, where, which model, and which key.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    OpenAiChat,
    AnthropicMessages,
}

impl Protocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiChat => "openai_chat",
            Self::AnthropicMessages => "anthropic_messages",
        }
    }
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Returned for an unknown protocol name. Does not echo the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the protocol must be openai_chat or anthropic_messages")]
pub struct UnknownProtocol;

impl FromStr for Protocol {
    type Err = UnknownProtocol;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "openai_chat" => Ok(Self::OpenAiChat),
            "anthropic_messages" => Ok(Self::AnthropicMessages),
            _ => Err(UnknownProtocol),
        }
    }
}
