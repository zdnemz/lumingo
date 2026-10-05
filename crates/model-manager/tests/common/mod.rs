#![allow(dead_code, clippy::expect_used, clippy::unwrap_used, clippy::panic)]
//! A small HTTP/1.1 server on the loopback address for the download tests.
//!
//! It serves byte bodies, honours `Range: bytes=N-`, and can misbehave in the
//! ways real servers do: cut the connection part way, ignore the range header,
//! redirect, answer an error status, or stop sending.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct Route {
    pub body: Vec<u8>,
    /// Answer `Range: bytes=N-` with 206. When false the header is ignored.
    pub honor_range: bool,
    /// Cut the next responses after this many body bytes, one count per cut.
    pub cut_after: Option<usize>,
    pub cuts_remaining: u32,
    /// Send this many body bytes and then send nothing more.
    pub stall_after: Option<usize>,
    pub status: Option<u16>,
    pub redirect_to: Option<String>,
}

impl Route {
    pub fn new(body: Vec<u8>) -> Self {
        Self {
            body,
            honor_range: true,
            cut_after: None,
            cuts_remaining: 0,
            stall_after: None,
            status: None,
            redirect_to: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub path: String,
    pub range: Option<String>,
    pub status: u16,
    pub body_bytes_sent: usize,
}

#[derive(Default)]
struct State {
    routes: HashMap<String, Route>,
    requests: Vec<Request>,
}

pub struct TestServer {
    addr: SocketAddr,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl TestServer {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.set_nonblocking(true).expect("nonblocking");
        let addr = listener.local_addr().expect("addr");
        let state = Arc::new(Mutex::new(State::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (s, st) = (Arc::clone(&state), Arc::clone(&stop));
        let handle = std::thread::spawn(move || {
            let mut workers = Vec::new();
            while !st.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let (s, st) = (Arc::clone(&s), Arc::clone(&st));
                        workers.push(std::thread::spawn(move || {
                            let _ = serve(stream, &s, &st);
                        }));
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(2)),
                }
            }
            for worker in workers {
                let _ = worker.join();
            }
        });
        Self {
            addr,
            state,
            stop,
            handle: Some(handle),
        }
    }

    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn set(&self, path: &str, route: Route) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .routes
            .insert(path.to_owned(), route);
    }

    pub fn update(&self, path: &str, change: impl FnOnce(&mut Route)) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        change(state.routes.get_mut(path).expect("route exists"));
    }

    pub fn requests(&self) -> Vec<Request> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .requests
            .clone()
    }

    pub fn request_count(&self) -> usize {
        self.requests().len()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn serve(mut stream: TcpStream, state: &Mutex<State>, stop: &AtomicBool) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut raw = Vec::new();
    let mut byte = [0_u8; 1];
    while !raw.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte)? == 0 {
            return Ok(());
        }
        raw.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&raw).into_owned();
    let mut lines = text.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let path = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .to_owned();
    let range = lines
        .filter_map(|l| l.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("range"))
        .map(|(_, v)| v.trim().to_owned());

    let route = state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .routes
        .get(&path)
        .cloned();
    let mut log = Request {
        path: path.clone(),
        range: range.clone(),
        status: 404,
        body_bytes_sent: 0,
    };
    let Some(route) = route else {
        write_simple(&mut stream, 404, "Not Found")?;
        record(state, log);
        return Ok(());
    };

    if let Some(target) = &route.redirect_to {
        log.status = 302;
        write!(
            stream,
            "HTTP/1.1 302 Found\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )?;
        record(state, log);
        return Ok(());
    }
    if let Some(status) = route.status {
        log.status = status;
        write_simple(&mut stream, status, "Error")?;
        record(state, log);
        return Ok(());
    }

    let total = route.body.len();
    let start = match (&range, route.honor_range) {
        (Some(r), true) => r
            .strip_prefix("bytes=")
            .and_then(|r| r.strip_suffix('-'))
            .and_then(|n| n.parse::<usize>().ok()),
        _ => None,
    };
    if let Some(start) = start
        && start >= total
    {
        log.status = 416;
        write!(
            stream,
            "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{total}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )?;
        record(state, log);
        return Ok(());
    }

    let (status_line, offset) = match start {
        Some(start) => ("206 Partial Content", start),
        None => ("200 OK", 0),
    };
    let body = &route.body[offset..];
    log.status = if start.is_some() { 206 } else { 200 };
    let mut head = format!(
        "HTTP/1.1 {status_line}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if start.is_some() {
        head.push_str(&format!(
            "Content-Range: bytes {offset}-{}/{total}\r\n",
            total - 1
        ));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;

    let mut limit = body.len();
    if route.cuts_remaining > 0
        && let Some(after) = route.cut_after
    {
        limit = after.min(body.len());
        let mut s = state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(r) = s.routes.get_mut(&path) {
            r.cuts_remaining -= 1;
        }
    }
    let mut stall = None;
    if let Some(after) = route.stall_after {
        limit = limit.min(after);
        stall = Some(after);
    }
    // Small writes so the client sees several chunks and can be interrupted
    // between them.
    for piece in body[..limit].chunks(16 * 1024) {
        stream.write_all(piece)?;
        log.body_bytes_sent += piece.len();
    }
    stream.flush()?;
    if stall.is_some() {
        let until = Instant::now() + Duration::from_secs(10);
        while !stop.load(Ordering::Acquire) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    record(state, log);
    // Dropping the stream closes the connection: for a cut response that is
    // before Content-Length bytes were sent.
    Ok(())
}

fn write_simple(stream: &mut TcpStream, status: u16, reason: &str) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
}

fn record(state: &Mutex<State>, request: Request) {
    state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .requests
        .push(request);
}

/// Deterministic bytes that do not compress to nothing and differ per seed.
pub fn pseudo_random(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (0..len)
        .map(|_| {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (x >> 33) as u8
        })
        .collect()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
