#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
// Each test binary uses a different subset of these helpers.
#![allow(dead_code)]

//! Shared helpers of the server tests: a harness that builds the whole router
//! over a real core in a temporary directory, and a request builder that can
//! leave out the Host, the Origin, the cookie or the content type.

use std::collections::HashMap;
use std::sync::Arc;

use app_core::config::CoreConfig;
use app_core::error::CoreResult;
use app_core::{AppCore, Clock};
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, Response, header};
use http_body_util::BodyExt;
use serde_json::Value;
use storage::{LocalDate, Timestamp};
use tempfile::TempDir;
use tower::ServiceExt;
use tutor_server::security::Guard;
use tutor_server::{AppState, build_router};

pub const PORT: u16 = 4321;
pub const HOST: &str = "127.0.0.1:4321";
pub const OWN_ORIGIN: &str = "http://127.0.0.1:4321";
pub const DEV_ORIGIN: &str = "http://localhost:3000";
pub const EVIL_ORIGIN: &str = "https://evil.example";

/// A clock the tests do not move: the server tests never depend on the date.
#[derive(Debug)]
pub struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        Timestamp::parse("2026-10-05T08:00:00.000Z").expect("timestamp")
    }

    fn today(&self) -> CoreResult<LocalDate> {
        Ok(LocalDate::parse("2026-10-05").expect("date"))
    }
}

pub struct Harness {
    pub router: Router,
    pub cookie: String,
    pub core: Arc<AppCore>,
    pub dir: TempDir,
}

/// What a test may choose about the core behind the router.
#[derive(Default)]
pub struct Options {
    pub dev: bool,
    /// Process environment of the core, for the `env` provider profile.
    pub env: Vec<(&'static str, &'static str)>,
    /// Copy the example unit into the unit folder before the core starts.
    pub with_unit: bool,
}

pub async fn harness(dev: bool) -> Harness {
    harness_with(Options {
        dev,
        ..Options::default()
    })
    .await
}

/// Opens a core in a temporary directory with a fixed clock, fixed hardware and
/// no `.env` files.
pub async fn open_core(options: &Options) -> (Arc<AppCore>, TempDir) {
    let dir = tempfile::tempdir().expect("temp dir");
    if options.with_unit {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../curriculum/examples/a1-u01.example.json");
        let units = dir.path().join("units");
        std::fs::create_dir_all(&units).expect("units dir");
        std::fs::copy(source, units.join("a1-u01.json")).expect("copy the example unit");
    }
    let mut config = CoreConfig::new(dir.path().join("data"), dir.path().join("units"));
    config.dev_mode = options.dev;
    config.server_address = Some(OWN_ORIGIN.to_owned());
    config.env_profiles = llm_client::EnvProfileLoader::new(Vec::new());
    let vars: HashMap<String, String> = options
        .env
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    config.env_lookup = Arc::new(move |name| vars.get(name).cloned());
    config.clock = Arc::new(FixedClock);
    config.hardware = Some(app_core::hardware::assess(Some(16_000_000_000), Some(8)));
    let core = AppCore::open(config).await.expect("open the core");
    (core, dir)
}

pub async fn harness_with(options: Options) -> Harness {
    let (core, dir) = open_core(&options).await;
    let dev_origin = options.dev.then(|| DEV_ORIGIN.to_owned());
    let guard = Arc::new(Guard::new(PORT, dev_origin).expect("guard"));
    let cookie = guard
        .set_cookie_value()
        .split(';')
        .next()
        .expect("cookie pair")
        .to_owned();
    let state = Arc::new(AppState::new(Arc::clone(&core), guard));
    Harness {
        router: build_router(state),
        cookie,
        core,
        dir,
    }
}

pub struct Req {
    pub method: Method,
    pub uri: String,
    pub host: Option<&'static str>,
    pub origin: Option<&'static str>,
    pub cookie: Option<String>,
    pub content_type: Option<&'static str>,
    pub websocket: bool,
    pub body: Option<String>,
}

impl Req {
    pub fn get(uri: &str) -> Self {
        Self {
            method: Method::GET,
            uri: uri.to_owned(),
            host: Some(HOST),
            origin: None,
            cookie: None,
            content_type: None,
            websocket: false,
            body: None,
        }
    }

    /// A state-changing request as the UI sends it: its own origin and JSON.
    /// The cookie is added by [`Req::authed`].
    pub fn json(method: Method, uri: &str, body: Option<Value>) -> Self {
        Self {
            method,
            uri: uri.to_owned(),
            host: Some(HOST),
            origin: Some(OWN_ORIGIN),
            cookie: None,
            content_type: Some("application/json"),
            websocket: false,
            body: body.map(|value| value.to_string()),
        }
    }

    pub fn post(uri: &str, body: Value) -> Self {
        Self::json(Method::POST, uri, Some(body))
    }

    pub fn put(uri: &str, body: Value) -> Self {
        Self::json(Method::PUT, uri, Some(body))
    }

    pub fn delete(uri: &str) -> Self {
        Self::json(Method::DELETE, uri, None)
    }

    /// Adds the session cookie.
    #[must_use]
    pub fn authed(mut self, cookie: &str) -> Self {
        self.cookie = Some(cookie.to_owned());
        self
    }

    pub fn build(self) -> Request<Body> {
        let mut builder = Request::builder().method(self.method).uri(self.uri);
        if let Some(host) = self.host {
            builder = builder.header(header::HOST, host);
        }
        if let Some(origin) = self.origin {
            builder = builder.header(header::ORIGIN, origin);
        }
        if let Some(cookie) = self.cookie {
            builder = builder.header(header::COOKIE, cookie);
        }
        if let Some(content_type) = self.content_type {
            builder = builder.header(header::CONTENT_TYPE, content_type);
        }
        if self.websocket {
            builder = builder
                .header(header::UPGRADE, "websocket")
                .header(header::CONNECTION, "Upgrade")
                .header("sec-websocket-version", "13")
                .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==");
        }
        let body = self.body.map_or_else(Body::empty, Body::from);
        builder.body(body).expect("request")
    }
}

pub async fn send(router: &Router, req: Req) -> Response<Body> {
    router.clone().oneshot(req.build()).await.expect("response")
}

pub async fn body_text(response: Response<Body>) -> String {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

pub async fn body_json(response: Response<Body>) -> Value {
    let text = body_text(response).await;
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"))
}

impl Harness {
    /// Sends an authorised GET and returns status and JSON body.
    pub async fn get_json(&self, uri: &str) -> (u16, Value) {
        let response = send(&self.router, Req::get(uri).authed(&self.cookie)).await;
        (response.status().as_u16(), body_json(response).await)
    }

    /// Sends an authorised state-changing request and returns status and JSON body.
    pub async fn call(&self, req: Req) -> (u16, Value) {
        let response = send(&self.router, req.authed(&self.cookie)).await;
        (response.status().as_u16(), body_json(response).await)
    }
}
