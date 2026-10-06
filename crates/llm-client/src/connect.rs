//! Building a working client from a profile.

use std::sync::Arc;

use crate::adapter::{AdapterConfig, ClientOptions, ProtocolAdapter};
use crate::anthropic::AnthropicMessages;
use crate::caps::CapsHandle;
use crate::error::LlmError;
use crate::http::HttpClientFactory;
use crate::inspector::PayloadLog;
use crate::ladder::ProviderClient;
use crate::openai::OpenAiChat;
use crate::profile::{Protocol, ProviderProfile};
use crate::types::Capabilities;

impl ProviderClient {
    /// Connects to the profile's provider through a new HTTP client factory that
    /// allows only the profile's host. `stored` carries capabilities from an earlier
    /// probe, so the learner is not probed on every start.
    pub fn connect(
        profile: &ProviderProfile,
        options: ClientOptions,
        stored: Option<Capabilities>,
    ) -> Result<Self, LlmError> {
        let factory = HttpClientFactory::new(&profile.base_url, options.tutor.connect)?;
        Self::connect_with_factory(profile, &factory, options, stored)
    }

    /// Like `connect`, and every request and response goes into `log`, for the
    /// payload inspector. The key is removed from what is recorded when it is
    /// captured.
    pub fn connect_logged(
        profile: &ProviderProfile,
        options: ClientOptions,
        stored: Option<Capabilities>,
        log: PayloadLog,
    ) -> Result<Self, LlmError> {
        let factory = HttpClientFactory::new(&profile.base_url, options.tutor.connect)?;
        Self::build(profile, &factory, options, stored, Some(log))
    }

    /// Like `connect`, with a factory the application keeps, so it can add model
    /// hosts during setup and switch the active provider.
    pub fn connect_with_factory(
        profile: &ProviderProfile,
        factory: &HttpClientFactory,
        options: ClientOptions,
        stored: Option<Capabilities>,
    ) -> Result<Self, LlmError> {
        Self::build(profile, factory, options, stored, None)
    }

    fn build(
        profile: &ProviderProfile,
        factory: &HttpClientFactory,
        options: ClientOptions,
        stored: Option<Capabilities>,
        log: Option<PayloadLog>,
    ) -> Result<Self, LlmError> {
        factory.set_active_provider(&profile.base_url)?;
        let caps = CapsHandle::new(stored.unwrap_or_default());
        let config = AdapterConfig {
            base_url: profile.base_url.clone(),
            model: profile.model.clone(),
            key: profile.key.clone(),
            http: factory.client(),
            caps: caps.clone(),
            options,
        };
        let adapter: Arc<dyn ProtocolAdapter> = match profile.protocol {
            Protocol::OpenAiChat => {
                let adapter = OpenAiChat::new(config)?;
                Arc::new(match log {
                    Some(log) => adapter.with_payload_log(log),
                    None => adapter,
                })
            }
            Protocol::AnthropicMessages => {
                let adapter = AnthropicMessages::new(config)?;
                Arc::new(match log {
                    Some(log) => adapter.with_payload_log(log),
                    None => adapter,
                })
            }
        };
        Ok(Self::from_adapter(adapter, caps))
    }
}
