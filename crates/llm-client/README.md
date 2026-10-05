# llm-client

The LLM client of Lumingo. Two wire protocols (`openai_chat`, `anthropic_messages`) behind the `LlmClient` trait, a capability probe, the structured-output ladder with local validation against `contracts/`, retries and limits, and keys held in memory only.

## Use

```rust
let profile = EnvProfileLoader::with_default_paths().load()?.ok_or(NoProfile)?; // read-only `env`
let client = ProviderClient::connect(&profile, ClientOptions::default(), stored_capabilities)?;
let capabilities = client.probe(&cancel).await?;                          // store it as JSON
let stream = client.stream_text(request, cancel.clone()).await?;          // tutor text
let analysis = client.structured(structured_request, cancel).await?;      // validated JSON
```

## Rules this crate keeps

- Every request goes through `HttpClientFactory`: only the active provider's host (plus setup hosts while a guard is held), HTTPS except for loopback, redirects only to allowed hosts, no system proxy.
- Timeouts: connect 5 s, first token 8 s, total 30 s for a tutor turn. Structured and probe calls have no first-token limit and 60 s in total.
- Retries: one for transport errors and 5xx with jitter; none for other 4xx; 429 follows `Retry-After`, up to two retries, and is not waited for when it would outlast the call.
- A key is an `ApiKey`: no `Display`, no `Serialize`, a redacting `Debug`. Errors carry fixed categories or sanitised provider text. The UI type is `ProfileInfo` (`has_key`, `key_last4`).
- `providers.toml` holds keys in plain text. It is written with mode 0600 on Unix. On Windows it inherits the permissions of the per-user data directory it is written into; no ACL call is made.

## Tests

`cargo test -p llm-client` replays hand-written fixtures (`tests/fixtures/`) through a scripted server on 127.0.0.1. No test contacts a provider.

## Not verified

- No provider has been called. The wire formats follow the providers' documentation as read on 2026-10-05; the owner runs the live probe against two providers (ROADMAP S3-03).
- The HTTPS path (native-tls) is compiled for Linux and checked for `x86_64-pc-windows-msvc`, but no test performs a TLS handshake, because the tests use loopback HTTP. Schannel on Windows is untested.
- The connect timeout is set on the HTTP client but no test reaches it.
- Rate-limit header names are the ones OpenAI and Anthropic document; other servers leave `rate_limit` empty.
