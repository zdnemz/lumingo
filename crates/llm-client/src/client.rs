use crate::{
    ApiKey, Level, LlmError, SseDecoder, SseEvent, StreamEvent, StructuredRequest, TextRequest,
    anthropic, extract_json,
    key::redact,
    openai::{self, TokenParam},
    policy::{self, Failure},
    structured, validate,
};
use reqwest::{Url, header::RETRY_AFTER};
use serde_json::Value;
use std::{
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    OpenaiChat,
    AnthropicMessages,
}

#[derive(Debug)]
pub struct ProviderConfig {
    pub protocol: Protocol,
    pub base_url: String,
    pub model: String,
    pub api_key: ApiKey,
}

/// context_pack.md section 13: connect 5 s, first token 8 s, total 30 s for a tutor turn.
#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    pub connect: Duration,
    pub first_token: Duration,
    pub total: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(5),
            first_token: Duration::from_secs(8),
            total: Duration::from_secs(30),
        }
    }
}

/// What the `openai_chat` probe or the first failing call taught us about this server.
#[derive(Debug, Clone, Copy)]
struct Quirks {
    token_param: TokenParam,
    send_temperature: bool,
    include_usage: bool,
}

/// One provider, one host. The endpoint is fixed at construction and redirects
/// are never followed, so a request cannot leave the configured host.
pub struct LlmClient {
    http: reqwest::Client,
    endpoint: Url,
    protocol: Protocol,
    model: String,
    key: ApiKey,
    timeouts: Timeouts,
    quirks: Mutex<Quirks>,
    structured_level: Mutex<Level>,
}

type Events = mpsc::Receiver<Result<StreamEvent, LlmError>>;

/// A validated structured answer and how it was obtained.
#[derive(Debug, Clone, PartialEq)]
pub struct StructuredOutput {
    pub value: Value,
    pub level: Level,
    /// True when the first answer was invalid and the repair call fixed it.
    pub repaired: bool,
}

/// What the probe learned, stored in `provider_profiles.capabilities_json`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Capabilities {
    pub probe_version: u32,
    pub auth_ok: bool,
    pub stream_ok: bool,
    pub ttft_ms: Option<u64>,
    pub tokens_per_second: Option<f64>,
    pub token_limit_param: String,
    pub supports_temperature: bool,
    pub supports_usage_in_stream: bool,
    /// `None` when no level produced valid JSON.
    pub structured_level: Option<u8>,
}

fn check_output(schema: &Value, text: &str) -> Result<Value, Vec<String>> {
    let value = extract_json(text)
        .ok_or_else(|| vec!["no complete JSON value was found in the reply".to_owned()])?;
    validate(schema, &value).map(|()| value)
}

impl LlmClient {
    pub fn new(cfg: ProviderConfig, timeouts: Timeouts) -> Result<Self, LlmError> {
        let base = policy::parse_base_url(&cfg.base_url)?;
        let host = base
            .host_str()
            .ok_or(LlmError::InvalidBaseUrl("no host"))?
            .to_owned();
        let suffix = match cfg.protocol {
            Protocol::OpenaiChat => "/chat/completions",
            Protocol::AnthropicMessages => "/v1/messages",
        };
        let endpoint = Url::parse(&format!("{}{suffix}", base.as_str().trim_end_matches('/')))
            .map_err(|_| LlmError::InvalidBaseUrl("not a URL"))?;
        policy::check_allowed_host(&endpoint, &[&host])?;
        let http = reqwest::Client::builder()
            .connect_timeout(timeouts.connect)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| LlmError::Transport(e.without_url().to_string()))?;
        Ok(Self {
            http,
            endpoint,
            protocol: cfg.protocol,
            model: cfg.model,
            key: cfg.api_key,
            timeouts,
            quirks: Mutex::new(Quirks {
                token_param: TokenParam::default(),
                send_temperature: true,
                include_usage: true,
            }),
            structured_level: Mutex::new(Level::NativeSchema),
        })
    }

    /// Starts a streamed reply. Errors before the first byte (auth, rate limit,
    /// network after one retry) come back here. Errors during the stream arrive
    /// on the channel, which is bounded: a slow reader pauses the download.
    pub async fn stream_text(
        &self,
        req: TextRequest,
        cancel: CancellationToken,
    ) -> Result<Events, LlmError> {
        let response = self.open(&|q| self.text_body(&req, q), &cancel).await?;
        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(pump(response, self.protocol, self.timeouts, cancel, tx));
        Ok(rx)
    }

    /// The ladder level structured calls start at. The probe sets it.
    pub fn structured_level(&self) -> Level {
        *self
            .structured_level
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_structured_level(&self, level: Level) {
        *self
            .structured_level
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = level;
    }

    /// A structured call at the cached ladder level (PROMPT_CONTRACTS section 5): the
    /// answer is extracted, validated, repaired once if needed, and otherwise
    /// reported as `InvalidOutput`. The caller applies its own semantic filters.
    pub async fn structured(
        &self,
        req: StructuredRequest,
        cancel: CancellationToken,
    ) -> Result<StructuredOutput, LlmError> {
        self.structured_at(&req, self.structured_level(), &cancel)
            .await
    }

    async fn structured_at(
        &self,
        req: &StructuredRequest,
        level: Level,
        cancel: &CancellationToken,
    ) -> Result<StructuredOutput, LlmError> {
        let first = self.call_level(req, level, cancel).await?;
        let errors = match check_output(&req.schema, &first) {
            Ok(value) => {
                return Ok(StructuredOutput {
                    value,
                    level,
                    repaired: false,
                });
            }
            Err(errors) => errors,
        };
        let second = self
            .call_level(&req.repair(&first, &errors), level, cancel)
            .await?;
        match check_output(&req.schema, &second) {
            Ok(value) => Ok(StructuredOutput {
                value,
                level,
                repaired: true,
            }),
            Err(_) => Err(LlmError::InvalidOutput {
                level: level.number(),
            }),
        }
    }

    /// One non-streaming request at `level`. A reply with nothing in the expected
    /// place comes back as empty text, which fails validation and triggers repair.
    async fn call_level(
        &self,
        req: &StructuredRequest,
        level: Level,
        cancel: &CancellationToken,
    ) -> Result<String, LlmError> {
        let make = |q: Quirks| match self.protocol {
            Protocol::OpenaiChat => structured::openai_body(req, level, q.token_param, &self.model),
            Protocol::AnthropicMessages => structured::anthropic_body(req, level, &self.model),
        };
        let response = self.open(&make, cancel).await?;
        let body = tokio::select! {
            () = cancel.cancelled() => return Err(LlmError::Cancelled),
            r = tokio::time::timeout(self.timeouts.total, response.text()) => match r {
                Err(_) => return Err(LlmError::Timeout("the reply body")),
                Ok(Err(e)) => return Err(LlmError::Transport(e.without_url().to_string())),
                Ok(Ok(text)) => text,
            },
        };
        let reply: Value =
            serde_json::from_str(&body).map_err(|_| crate::ParseError::InvalidJson)?;
        Ok(structured::output_text(self.protocol, level, &reply).unwrap_or_default())
    }

    /// The connection test (PROMPT_CONTRACTS section 4). One short streaming call
    /// checks the key and measures speed (the spec's steps 1 and 2 in one request),
    /// then structured calls with a three-field schema walk the ladder until one
    /// level returns valid JSON, and that level is cached. Warming the contract
    /// schemas and reading rate-limit headers (steps 4 and 5) are not done yet.
    pub async fn probe(&self, cancel: CancellationToken) -> Capabilities {
        let mut caps = Capabilities {
            probe_version: 1,
            ..Capabilities::default()
        };
        let started = Instant::now();
        let hello = TextRequest {
            model: self.model.clone(),
            system: None,
            messages: vec![crate::Message {
                role: crate::Role::User,
                content: "Reply with one short sentence.".into(),
            }],
            max_tokens: 40,
            temperature: Some(0.0),
        };
        match self.stream_text(hello, cancel.clone()).await {
            Ok(mut rx) => {
                caps.auth_ok = true;
                let mut first_token = None;
                while let Some(event) = rx.recv().await {
                    match event {
                        Ok(StreamEvent::Text(_)) => {
                            first_token.get_or_insert_with(|| started.elapsed());
                            caps.stream_ok = true;
                        }
                        Ok(StreamEvent::Usage(u)) => {
                            if let (Some(n), Some(t)) = (u.output_tokens, first_token) {
                                let secs = started.elapsed().saturating_sub(t).as_secs_f64();
                                if secs > 0.0 && n > 1 {
                                    caps.tokens_per_second = Some((n - 1) as f64 / secs);
                                }
                            }
                            caps.supports_usage_in_stream |= u.output_tokens.is_some();
                        }
                        Ok(_) => {}
                        Err(_) => {
                            caps.stream_ok = false;
                            break;
                        }
                    }
                }
                caps.ttft_ms = first_token.map(|t| t.as_millis() as u64);
            }
            Err(LlmError::Auth(_)) => return caps,
            Err(_) => {}
        }
        let quirks = self.quirks();
        caps.token_limit_param = quirks.token_param.key().to_owned();
        caps.supports_temperature = quirks.send_temperature;
        caps.supports_usage_in_stream = caps.supports_usage_in_stream && quirks.include_usage;

        let test = StructuredRequest {
            system: None,
            messages: vec![crate::Message {
                role: crate::Role::User,
                content: "Give the word \"tea\", its length, and whether it is a noun.".into(),
            }],
            schema_name: "probe_check".into(),
            schema: serde_json::json!({
                "type": "object",
                "properties": { "word": { "type": "string" }, "length": { "type": "integer" }, "is_noun": { "type": "boolean" } },
                "required": ["word", "length", "is_noun"],
                "additionalProperties": false
            }),
            max_tokens: 100,
        };
        let mut level = Some(Level::NativeSchema);
        while let Some(l) = level {
            match self.structured_at(&test, l, &cancel).await {
                Ok(out) => {
                    caps.structured_level = Some(out.level.number());
                    self.set_structured_level(out.level);
                    break;
                }
                // These say nothing about the level: stop instead of walking down.
                Err(
                    LlmError::Auth(_)
                    | LlmError::RateLimited { .. }
                    | LlmError::Cancelled
                    | LlmError::Transport(_)
                    | LlmError::Timeout(_),
                ) => break,
                Err(_) => level = l.next(self.protocol),
            }
        }
        caps
    }

    fn quirks(&self) -> Quirks {
        *self.quirks.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn text_body(&self, req: &TextRequest, q: Quirks) -> Value {
        let mut req = req.clone();
        req.model.clone_from(&self.model);
        if !q.send_temperature {
            req.temperature = None;
        }
        match self.protocol {
            Protocol::OpenaiChat => openai::request_body(&req, q.token_param, q.include_usage),
            Protocol::AnthropicMessages => anthropic::request_body(&req),
        }
    }

    fn post(&self, body: &Value) -> reqwest::RequestBuilder {
        let builder = self.http.post(self.endpoint.clone());
        match self.protocol {
            Protocol::OpenaiChat => builder.bearer_auth(self.key.expose()),
            Protocol::AnthropicMessages => builder
                .header("x-api-key", self.key.expose())
                .header("anthropic-version", "2023-06-01"),
        }
        .json(body)
    }

    /// Sends the request, retrying as `policy::retry_wait` allows and adapting to
    /// `openai_chat` servers that reject a parameter we sent.
    async fn open(
        &self,
        make_body: &dyn Fn(Quirks) -> Value,
        cancel: &CancellationToken,
    ) -> Result<reqwest::Response, LlmError> {
        let (mut attempt, mut adjustments) = (0u32, 0u32);
        loop {
            let sent = tokio::select! {
                () = cancel.cancelled() => return Err(LlmError::Cancelled),
                r = tokio::time::timeout(self.timeouts.total, self.post(&make_body(self.quirks())).send()) => r,
            };
            let (failure, error) = match sent {
                Err(_) => (Failure::Transport, LlmError::Timeout("the response")),
                Ok(Err(e)) => (
                    Failure::Transport,
                    LlmError::Transport(redact(&e.without_url().to_string(), &self.key)),
                ),
                Ok(Ok(resp)) if resp.status().is_success() => return Ok(resp),
                Ok(Ok(resp)) => {
                    let code = resp.status().as_u16();
                    let retry_after = resp
                        .headers()
                        .get(RETRY_AFTER)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.trim().parse::<u64>().ok())
                        .map(Duration::from_secs);
                    let body = resp.text().await.unwrap_or_default();
                    if code == 400
                        && self.protocol == Protocol::OpenaiChat
                        && adjustments < 3
                        && self.adjust(&body)
                    {
                        adjustments += 1;
                        continue;
                    }
                    let message = redact(&body, &self.key);
                    let error = match code {
                        401 | 403 => LlmError::Auth(code),
                        429 => LlmError::RateLimited { retry_after },
                        _ => LlmError::Http {
                            status: code,
                            message,
                        },
                    };
                    (Failure::Status { code, retry_after }, error)
                }
            };
            let Some(wait) = policy::retry_wait(attempt, failure, jitter()) else {
                return Err(error);
            };
            attempt += 1;
            tokio::select! {
                () = cancel.cancelled() => return Err(LlmError::Cancelled),
                () = tokio::time::sleep(wait) => {}
            }
        }
    }

    /// A 400 that names a parameter we sent: stop sending it (or send the other
    /// token-limit name) and remember. Returns whether anything changed.
    fn adjust(&self, body: &str) -> bool {
        let body = body.to_lowercase();
        let mut q = self.quirks.lock().unwrap_or_else(|e| e.into_inner());
        if q.token_param == TokenParam::MaxTokens && body.contains("max_tokens") {
            q.token_param = TokenParam::MaxCompletionTokens;
        } else if q.token_param == TokenParam::MaxCompletionTokens
            && body.contains("max_completion_tokens")
        {
            q.token_param = TokenParam::MaxTokens;
        } else if q.send_temperature && body.contains("temperature") {
            q.send_temperature = false;
        } else if q.include_usage
            && (body.contains("stream_options") || body.contains("include_usage"))
        {
            q.include_usage = false;
        } else {
            return false;
        }
        true
    }
}

fn jitter() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.5, |d| f64::from(d.subsec_nanos()) / 1e9)
}

fn parse(protocol: Protocol, event: &SseEvent) -> Result<Vec<StreamEvent>, crate::ParseError> {
    match protocol {
        Protocol::OpenaiChat => openai::parse_event(event),
        Protocol::AnthropicMessages => anthropic::parse_event(event),
    }
}

/// Reads the body, decodes it and forwards events until the stream ends, an error
/// event arrives, the reader goes away, or a timeout or cancel fires.
async fn pump(
    mut response: reqwest::Response,
    protocol: Protocol,
    timeouts: Timeouts,
    cancel: CancellationToken,
    tx: mpsc::Sender<Result<StreamEvent, LlmError>>,
) {
    let (start, mut decoder, mut got_text) = (Instant::now(), SseDecoder::default(), false);
    loop {
        let left = timeouts.total.saturating_sub(start.elapsed());
        let (wait, what) = if got_text {
            (left, "the rest of the reply")
        } else {
            (left.min(timeouts.first_token), "the first token")
        };
        let (events, done) = tokio::select! {
            () = cancel.cancelled() => { let _ = tx.send(Err(LlmError::Cancelled)).await; return; }
            r = tokio::time::timeout(wait, response.chunk()) => match r {
                Err(_) => { let _ = tx.send(Err(LlmError::Timeout(what))).await; return; }
                Ok(Err(e)) => { let _ = tx.send(Err(LlmError::Transport(e.without_url().to_string()))).await; return; }
                Ok(Ok(Some(bytes))) => (decoder.push(&bytes), false),
                Ok(Ok(None)) => (decoder.finish(), true),
            },
        };
        for event in &events {
            match parse(protocol, event) {
                Ok(parsed) => {
                    for p in parsed {
                        got_text |= matches!(p, StreamEvent::Text(_));
                        let is_error = matches!(p, StreamEvent::Error(_));
                        if tx.send(Ok(p)).await.is_err() || is_error {
                            return;
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e.into())).await;
                    return;
                }
            }
        }
        if done {
            return;
        }
    }
}
