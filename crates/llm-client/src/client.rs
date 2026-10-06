use crate::{
    ApiKey, LlmError, SseDecoder, SseEvent, StreamEvent, TextRequest, anthropic,
    key::redact,
    openai::{self, TokenParam},
    policy::{self, Failure},
};
use reqwest::{Url, header::RETRY_AFTER};
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
}

type Events = mpsc::Receiver<Result<StreamEvent, LlmError>>;

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
        let response = self.open(&req, &cancel).await?;
        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(pump(response, self.protocol, self.timeouts, cancel, tx));
        Ok(rx)
    }

    fn quirks(&self) -> Quirks {
        *self.quirks.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn request(&self, req: &TextRequest, q: Quirks) -> reqwest::RequestBuilder {
        let mut req = req.clone();
        req.model.clone_from(&self.model);
        if !q.send_temperature {
            req.temperature = None;
        }
        match self.protocol {
            Protocol::OpenaiChat => self
                .http
                .post(self.endpoint.clone())
                .bearer_auth(self.key.expose())
                .json(&openai::request_body(&req, q.token_param, q.include_usage)),
            Protocol::AnthropicMessages => self
                .http
                .post(self.endpoint.clone())
                .header("x-api-key", self.key.expose())
                .header("anthropic-version", "2023-06-01")
                .json(&anthropic::request_body(&req)),
        }
    }

    /// Sends the request, retrying as `policy::retry_wait` allows and adapting to
    /// `openai_chat` servers that reject a parameter we sent.
    async fn open(
        &self,
        req: &TextRequest,
        cancel: &CancellationToken,
    ) -> Result<reqwest::Response, LlmError> {
        let (mut attempt, mut adjustments) = (0u32, 0u32);
        loop {
            let sent = tokio::select! {
                () = cancel.cancelled() => return Err(LlmError::Cancelled),
                r = tokio::time::timeout(self.timeouts.total, self.request(req, self.quirks()).send()) => r,
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
