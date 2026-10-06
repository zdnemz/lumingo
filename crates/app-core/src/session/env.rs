//! What every kind of session is built from: the database, the clock, the
//! language model of the active provider, the learner's settings.

use std::sync::Arc;

use curriculum::Level as UnitLevel;
use llm_client::LlmClient;
use storage::Database;
use tutor_engine::{Clock, NoProvider};

use crate::api::Feature;
use crate::core::AppCore;
use crate::error::{CoreError, CoreResult};

/// The learner's first language as the prompts want it: written in English.
/// A code this table does not know is passed on as it is, which the model
/// understands better than a guess.
pub(crate) fn language_name(code: &str) -> String {
    let name = match code.trim().to_ascii_lowercase().as_str() {
        "id" => "Indonesian",
        "en" => "English",
        "ms" => "Malay",
        "jv" => "Javanese",
        "su" => "Sundanese",
        "zh" => "Chinese",
        "ja" => "Japanese",
        "ko" => "Korean",
        "vi" => "Vietnamese",
        "th" => "Thai",
        "tl" => "Filipino",
        "hi" => "Hindi",
        "bn" => "Bengali",
        "ar" => "Arabic",
        "tr" => "Turkish",
        "ru" => "Russian",
        "uk" => "Ukrainian",
        "pl" => "Polish",
        "de" => "German",
        "fr" => "French",
        "es" => "Spanish",
        "pt" => "Portuguese",
        "it" => "Italian",
        "nl" => "Dutch",
        "sv" => "Swedish",
        "fa" => "Persian",
        "ur" => "Urdu",
        "sw" => "Swahili",
        other => return other.to_owned(),
    };
    name.to_owned()
}

/// The assessment crate's level for the API's level pick.
pub(crate) fn assessment_level(level: UnitLevel) -> assessment_engine::Level {
    match level {
        UnitLevel::A1 => assessment_engine::Level::A1,
        UnitLevel::A2 => assessment_engine::Level::A2,
        UnitLevel::B1 => assessment_engine::Level::B1,
        UnitLevel::B2 => assessment_engine::Level::B2,
        UnitLevel::C1 => assessment_engine::Level::C1,
        UnitLevel::C2 => assessment_engine::Level::C2,
    }
}

/// The curriculum's level for the assessment crate's.
pub(crate) fn unit_level(level: assessment_engine::Level) -> UnitLevel {
    match level {
        assessment_engine::Level::A1 => UnitLevel::A1,
        assessment_engine::Level::A2 => UnitLevel::A2,
        assessment_engine::Level::B1 => UnitLevel::B1,
        assessment_engine::Level::B2 => UnitLevel::B2,
        assessment_engine::Level::C1 => UnitLevel::C1,
        assessment_engine::Level::C2 => UnitLevel::C2,
    }
}

/// Everything a session is built from, read when the session starts.
#[derive(Clone)]
pub(crate) struct Env {
    pub core: Arc<AppCore>,
    pub db: Database,
    pub clock: Clock,
    pub llm: Arc<dyn LlmClient>,
    /// The model name stored with every analysis. `none` without a provider.
    pub model: String,
    pub provider_profile_id: Option<i64>,
    pub provider_qualified: bool,
    pub profile_id: i64,
    pub first_language: String,
    pub app_version: String,
}

impl Env {
    /// Builds the environment. With `need_provider`, a missing provider is an
    /// error (`ProviderNotConfigured`); without it the client is [`NoProvider`],
    /// so a lesson or a writing draft works and its productive responses wait.
    /// `override_client` replaces the provider's client (tests only).
    pub(crate) async fn load(
        core: &Arc<AppCore>,
        need_provider: bool,
        override_client: Option<Arc<dyn LlmClient>>,
    ) -> CoreResult<Self> {
        let active = core.providers.active();
        // Without a provider, a session that can carry on without one gets a client
        // that fails at once and says why, so the offline paths take over.
        let llm: Arc<dyn LlmClient> = match override_client {
            Some(client) => client,
            None => match core.llm_client().await {
                Ok(client) => client,
                Err(CoreError::ProviderNotConfigured) if !need_provider => Arc::new(NoProvider),
                Err(error) => return Err(error),
            },
        };
        let clock_source = Arc::clone(&core.config.clock);
        let clock: Clock = Arc::new(move || clock_source.now());
        let settings = core.settings();
        Ok(Self {
            core: Arc::clone(core),
            db: core.db.clone(),
            clock,
            llm,
            model: active
                .as_ref()
                .map_or_else(|| "none".to_owned(), |p| p.model.clone()),
            provider_profile_id: active.as_ref().map(|p| p.id),
            provider_qualified: active.as_ref().is_some_and(|p| p.qualified_at.is_some()),
            profile_id: core.profile_id(),
            first_language: language_name(&settings.l1),
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
        })
    }
}

/// What a failure of an engine call is to the API. The messages of the engines
/// hold no learner text, no prompt and no key.
pub(crate) fn engine_error(error: tutor_engine::EngineError) -> CoreError {
    use tutor_engine::EngineError;
    match error {
        EngineError::Storage(error) => CoreError::Storage(error),
        EngineError::Llm(llm_client::LlmError::InvalidRequest(message))
            if message == "no provider is configured" =>
        {
            CoreError::ProviderNotConfigured
        }
        EngineError::Llm(llm_client::LlmError::Cancelled) | EngineError::Cancelled => {
            CoreError::Conflict("the call was cancelled".to_owned())
        }
        EngineError::Llm(error) => CoreError::Conflict(format!(
            "the language model provider could not be used: {error}"
        )),
        EngineError::Transition(error) => CoreError::Conflict(error.to_string()),
        EngineError::Activity(error) => CoreError::InvalidInput(error.to_string()),
        EngineError::Output(contract) => {
            CoreError::Internal(format!("the {contract} output did not fit its typed form"))
        }
        EngineError::Refused(message) => CoreError::InvalidInput(message.to_owned()),
    }
}

/// `NotAvailable` for a kind of session whose engine does not exist, naming it.
pub(crate) fn missing_engine(what: &str) -> CoreError {
    CoreError::unavailable(None::<Feature>, what)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_codes_become_english_names_and_unknown_ones_pass_through() {
        assert_eq!(language_name("id"), "Indonesian");
        assert_eq!(language_name(" EN "), "English");
        assert_eq!(language_name("xx"), "xx");
    }

    #[test]
    fn levels_convert_both_ways() {
        for level in assessment_engine::Level::ALL {
            assert_eq!(assessment_level(unit_level(level)), level);
        }
    }
}
