#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
//! Retries, timeouts and the allowlist over real HTTP (`context_pack.md` section 13,
//! PRD NFR-S1).

mod common;

use std::time::{Duration, Instant};

use common::{
    Chunk, Reply, TEST_KEY, TestServer, collect, complete, fixture, openai_rig, quick_options, rig,
    text_request,
};
use futures_util::StreamExt;
use llm_client::{
    AdapterConfig, Capabilities, CapsHandle, Format, HttpClientFactory, LlmError, OpenAiChat,
    ProtocolAdapter, StreamEvent, TimeoutKind,
};
use reqwest::Url;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

fn ok_stream() -> Reply {
    Reply::sse_fixture("openai/stream_basic.sse")
}

// ---- 429 ---------------------------------------------------------------------------

#[tokio::test]
async fn a_429_with_retry_after_waits_that_long_and_retries() {
    let (rig, adapter) = openai_rig().await;
    rig.server
        .enqueue(Reply::empty(429).with_header("retry-after", "1"));
    rig.server.enqueue(ok_stream());

    let started = Instant::now();
    let collected = collect(&adapter, &text_request()).await.expect("retried");

    assert_eq!(collected.text, "Hello, nice to meet you.");
    assert_eq!(rig.server.hits(), 2);
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_millis(950) && waited < Duration::from_millis(2500),
        "{waited:?}"
    );
}

#[tokio::test]
async fn a_429_without_retry_after_backs_off_exponentially_for_two_retries() {
    let (rig, adapter) = openai_rig().await;
    // The test policy starts at 60 ms: waits of 60 ms and 120 ms.
    rig.server.enqueue(Reply::empty(429));
    rig.server.enqueue(Reply::empty(429));
    rig.server.enqueue(ok_stream());

    let started = Instant::now();
    collect(&adapter, &text_request()).await.expect("third try");

    assert_eq!(rig.server.hits(), 3);
    assert!(
        started.elapsed() >= Duration::from_millis(170),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_third_429_is_returned_as_rate_limited() {
    let (rig, adapter) = openai_rig().await;
    for _ in 0..3 {
        rig.server
            .enqueue(Reply::empty(429).with_header("retry-after", "0"));
    }

    let error = collect(&adapter, &text_request())
        .await
        .expect_err("limited");

    assert!(
        matches!(error, LlmError::RateLimited { retry_after: Some(d) } if d == Duration::ZERO),
        "{error:?}"
    );
    assert_eq!(rig.server.hits(), 3, "the first try and two retries");
}

#[tokio::test]
async fn a_retry_after_longer_than_the_turn_budget_is_not_waited_for() {
    let (rig, adapter) = openai_rig().await;
    rig.server
        .enqueue(Reply::empty(429).with_header("retry-after", "120"));

    let started = Instant::now();
    let error = collect(&adapter, &text_request())
        .await
        .expect_err("limited");

    assert!(
        matches!(error, LlmError::RateLimited { retry_after: Some(d) } if d == Duration::from_secs(120)),
        "{error:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(rig.server.hits(), 1, "no retry that cannot finish in time");
}

#[tokio::test]
async fn cancelling_during_the_wait_for_a_retry_returns_cancelled_quickly() {
    let (rig, adapter) = openai_rig().await;
    rig.server
        .enqueue(Reply::empty(429).with_header("retry-after", "3"));
    let cancel = CancellationToken::new();
    let canceller = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        canceller.cancel();
    });

    let started = Instant::now();
    let error = adapter
        .stream_text(&text_request(), &cancel)
        .await
        .expect_err("cancelled");

    assert!(matches!(error, LlmError::Cancelled), "{error:?}");
    assert!(started.elapsed() < Duration::from_millis(500));
}

// ---- 5xx and transport errors ------------------------------------------------------

#[tokio::test]
async fn a_5xx_is_retried_once_with_jitter() {
    let (rig, adapter) = openai_rig().await;
    rig.server.enqueue(Reply::empty(503));
    rig.server.enqueue(ok_stream());

    collect(&adapter, &text_request()).await.expect("retried");

    assert_eq!(rig.server.hits(), 2);
}

#[tokio::test]
async fn a_second_5xx_is_returned() {
    let (rig, adapter) = openai_rig().await;
    rig.server.enqueue(Reply::empty(502));
    rig.server.enqueue(Reply::empty(500));

    let error = collect(&adapter, &text_request()).await.expect_err("down");

    assert!(
        matches!(error, LlmError::Server { status: 500, .. }),
        "{error:?}"
    );
    assert_eq!(rig.server.hits(), 2, "one retry, not more");
}

#[tokio::test]
async fn a_4xx_other_than_429_is_never_retried() {
    for (status, fixture_name) in [
        (400, "openai/error_model_not_found.json"),
        (401, "openai/error_incorrect_key.json"),
        (403, "openai/error_incorrect_key.json"),
        (404, "openai/error_model_not_found.json"),
        (422, "openai/error_model_not_found.json"),
    ] {
        let (rig, adapter) = openai_rig().await;
        rig.server
            .enqueue(Reply::json_fixture(status, fixture_name));
        rig.server.enqueue(ok_stream());

        let result = collect(&adapter, &text_request()).await;

        assert!(result.is_err(), "{status}");
        assert_eq!(rig.server.hits(), 1, "HTTP {status} must not be retried");
    }
}

/// A server that drops the first connection without answering, then serves one reply.
async fn flaky_server(reply_body: Vec<u8>) -> (Url, tokio::task::JoinHandle<usize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let url = Url::parse(&format!(
        "http://{}/v1",
        listener.local_addr().expect("addr")
    ))
    .expect("url");
    let handle = tokio::spawn(async move {
        let mut accepted = 0;
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return accepted;
            };
            accepted += 1;
            let mut buffer = vec![0u8; 8192];
            let _ = socket.read(&mut buffer).await;
            if accepted == 1 {
                drop(socket);
                continue;
            }
            let head = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                reply_body.len()
            );
            let _ = socket.write_all(head.as_bytes()).await;
            let _ = socket.write_all(&reply_body).await;
            let _ = socket.shutdown().await;
        }
    });
    (url, handle)
}

#[tokio::test]
async fn a_dropped_connection_is_retried_once() {
    let (url, server) = flaky_server(fixture("openai/stream_basic.sse")).await;
    let options = quick_options();
    let factory = HttpClientFactory::new(&url, options.tutor.connect).expect("factory");
    let adapter = OpenAiChat::new(AdapterConfig {
        base_url: url,
        model: "m".to_owned(),
        key: None,
        http: factory.client(),
        caps: CapsHandle::new(Capabilities::default()),
        options,
    })
    .expect("adapter");

    let collected = collect(&adapter, &text_request()).await.expect("retried");

    assert_eq!(collected.text, "Hello, nice to meet you.");
    server.abort();
}

#[tokio::test]
async fn a_server_that_is_not_there_is_a_transport_error_after_one_retry() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let url = Url::parse(&format!(
        "http://{}/v1",
        listener.local_addr().expect("addr")
    ))
    .expect("url");
    drop(listener);
    let options = quick_options();
    let factory = HttpClientFactory::new(&url, options.tutor.connect).expect("factory");
    let adapter = OpenAiChat::new(AdapterConfig {
        base_url: url,
        model: "m".to_owned(),
        key: None,
        http: factory.client(),
        caps: CapsHandle::new(Capabilities::default()),
        options,
    })
    .expect("adapter");

    let error = collect(&adapter, &text_request())
        .await
        .expect_err("refused");

    assert!(matches!(error, LlmError::Transport(_)), "{error:?}");
    assert!(error.is_provider_unavailable());
}

// ---- timeouts ----------------------------------------------------------------------

#[tokio::test]
async fn first_token_limit_applies_to_a_slow_start() {
    let mut options = quick_options();
    options.tutor.first_token = Some(Duration::from_millis(300));
    let rig = rig("/v1", Some(TEST_KEY), options).await;
    let adapter = OpenAiChat::new(rig.config.clone()).expect("adapter");
    rig.server.enqueue(Reply::sse_chunks(vec![Chunk {
        delay: Duration::from_millis(1500),
        bytes: fixture("openai/stream_basic.sse"),
    }]));

    let started = Instant::now();
    let error = match adapter
        .stream_text(&text_request(), &CancellationToken::new())
        .await
    {
        Ok(stream) => stream.collect_text().await.expect_err("too slow"),
        Err(error) => error,
    };

    assert!(
        matches!(error, LlmError::Timeout(TimeoutKind::FirstToken)),
        "{error:?}"
    );
    assert!(started.elapsed() < Duration::from_millis(900));
}

#[tokio::test]
async fn keep_alive_comments_do_not_count_as_the_first_token() {
    let mut options = quick_options();
    options.tutor.first_token = Some(Duration::from_millis(400));
    let rig = rig("/v1", Some(TEST_KEY), options).await;
    let adapter = OpenAiChat::new(rig.config.clone()).expect("adapter");
    rig.server.enqueue(Reply::sse_chunks(vec![
        Chunk {
            delay: Duration::ZERO,
            bytes: b": keep-alive\n\n".to_vec(),
        },
        Chunk {
            delay: Duration::from_millis(200),
            bytes: b": keep-alive\n\n".to_vec(),
        },
        Chunk {
            delay: Duration::from_millis(1000),
            bytes: b"data: {\"choices\":[{\"delta\":{\"content\":\"late\"}}]}\n\n".to_vec(),
        },
    ]));

    let error = collect(&adapter, &text_request())
        .await
        .expect_err("no content in time");

    assert!(
        matches!(error, LlmError::Timeout(TimeoutKind::FirstToken)),
        "{error:?}"
    );
}

#[tokio::test]
async fn the_total_limit_ends_a_stream_that_stalls_after_the_first_token() {
    let mut options = quick_options();
    options.tutor.first_token = Some(Duration::from_millis(500));
    options.tutor.total = Duration::from_millis(900);
    let rig = rig("/v1", Some(TEST_KEY), options).await;
    let adapter = OpenAiChat::new(rig.config.clone()).expect("adapter");
    let first = "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n";
    rig.server.enqueue(Reply::sse_chunks(vec![
        Chunk {
            delay: Duration::ZERO,
            bytes: first.as_bytes().to_vec(),
        },
        Chunk {
            delay: Duration::from_secs(30),
            bytes: b"data: [DONE]\n\n".to_vec(),
        },
    ]));

    let mut stream = adapter
        .stream_text(&text_request(), &CancellationToken::new())
        .await
        .expect("stream");
    assert_eq!(
        stream.next().await.expect("item").expect("delta"),
        StreamEvent::Delta("Hel".to_owned())
    );
    let started = Instant::now();
    let error = stream.next().await.expect("item").expect_err("timeout");

    assert!(
        matches!(error, LlmError::Timeout(TimeoutKind::Total)),
        "{error:?}"
    );
    assert!(started.elapsed() < Duration::from_millis(1200));
}

#[tokio::test]
async fn the_total_limit_covers_a_non_streaming_reply() {
    let mut options = quick_options();
    options.background.total = Duration::from_millis(500);
    let rig = rig("/v1", Some(TEST_KEY), options).await;
    let adapter = OpenAiChat::new(rig.config.clone()).expect("adapter");
    rig.server.enqueue(Reply::sse_chunks(vec![Chunk {
        delay: Duration::from_secs(10),
        bytes: b"{}".to_vec(),
    }]));

    let error = complete(&adapter, |_| Format::Plain, None)
        .await
        .expect_err("timeout");

    assert!(matches!(error, LlmError::Timeout(_)), "{error:?}");
}

#[test]
fn the_default_limits_are_the_documented_ones() {
    let options = llm_client::ClientOptions::default();
    assert_eq!(options.tutor.connect, Duration::from_secs(5));
    assert_eq!(options.tutor.first_token, Some(Duration::from_secs(8)));
    assert_eq!(options.tutor.total, Duration::from_secs(30));
    assert_eq!(options.retry.transport_retries, 1);
    assert_eq!(options.retry.rate_limit_retries, 2);
    assert_eq!(options.retry.rate_limit_backoff, Duration::from_secs(2));
}

// ---- allowlist and HTTPS rule ------------------------------------------------------

#[tokio::test]
async fn a_forbidden_host_is_refused_without_a_connection() {
    let allowed = TestServer::start().await;
    let forbidden = TestServer::start().await;
    let factory =
        HttpClientFactory::new(&allowed.url("/v1"), Duration::from_secs(1)).expect("factory");
    let client = factory.client();

    let refused = client.get(&forbidden.url("/models"));

    assert!(
        matches!(refused, Err(LlmError::HostNotAllowed { .. })),
        "{refused:?}"
    );
    assert_eq!(
        forbidden.hits(),
        0,
        "no socket may be opened to a forbidden host"
    );
    // The allowed host works through the same client.
    allowed.enqueue(Reply::empty(200));
    let response = client
        .get(&allowed.url("/v1/models"))
        .expect("allowed")
        .send()
        .await
        .expect("sent");
    assert!(response.status().is_success());
}

#[tokio::test]
async fn a_redirect_to_another_host_is_not_followed() {
    let provider = TestServer::start().await;
    let elsewhere = TestServer::start().await;
    let options = quick_options();
    let factory =
        HttpClientFactory::new(&provider.url("/v1"), options.tutor.connect).expect("factory");
    let adapter = OpenAiChat::new(AdapterConfig {
        base_url: provider.url("/v1"),
        model: "m".to_owned(),
        key: llm_client::ApiKey::new(TEST_KEY).ok(),
        http: factory.client(),
        caps: CapsHandle::new(Capabilities::default()),
        options,
    })
    .expect("adapter");
    provider.enqueue(Reply::empty(307).with_header("location", elsewhere.url("/steal").as_str()));

    let error = collect(&adapter, &text_request())
        .await
        .expect_err("refused");

    assert!(
        matches!(error, LlmError::HostNotAllowed { .. }),
        "{error:?}"
    );
    assert_eq!(
        elsewhere.hits(),
        0,
        "the key must not follow a redirect off the allowlist"
    );
}

#[tokio::test]
async fn plain_http_to_a_non_loopback_host_is_refused_everywhere() {
    // Building a factory for such a base URL fails.
    let remote = Url::parse("http://api.example.com/v1").expect("url");
    assert!(matches!(
        HttpClientFactory::new(&remote, Duration::from_secs(1)),
        Err(LlmError::InsecureScheme { .. })
    ));
    // A client built for an HTTPS provider refuses the HTTP form of the same host.
    let https = Url::parse("https://api.example.com/v1").expect("url");
    let factory = HttpClientFactory::new(&https, Duration::from_secs(1)).expect("factory");
    let refused = factory.client().post(&remote);
    assert!(
        matches!(refused, Err(LlmError::InsecureScheme { .. })),
        "{refused:?}"
    );
    // Model hosts for setup need HTTPS as well.
    assert!(
        factory
            .allow_setup_hosts(&[Url::parse("http://models.example.com").expect("url")])
            .is_err()
    );
}

#[tokio::test]
async fn setup_hosts_are_reachable_only_while_the_guard_is_held() {
    let provider = TestServer::start().await;
    let models = TestServer::start().await;
    let factory =
        HttpClientFactory::new(&provider.url("/v1"), Duration::from_secs(1)).expect("factory");
    let client = factory.client();
    models.enqueue(Reply::empty(200));

    assert!(client.get(&models.url("/file")).is_err());
    let guard = factory
        .allow_setup_hosts(&[models.url("/")])
        .expect("allow");
    let response = client
        .get(&models.url("/file"))
        .expect("allowed during setup")
        .send()
        .await
        .expect("sent");
    assert!(response.status().is_success());
    drop(guard);

    assert!(matches!(
        client.get(&models.url("/file")),
        Err(LlmError::HostNotAllowed { .. })
    ));
}
