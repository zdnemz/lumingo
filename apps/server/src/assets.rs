//! The web UI, embedded in the executable.
//!
//! In a release build `apps/web/out` is compiled into the binary. In a debug
//! build the files are read from disk, so a rebuilt UI shows up without
//! recompiling the server.

use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::response::Response;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rust_embed::RustEmbed;
use sha2::{Digest, Sha256};

#[derive(RustEmbed)]
#[folder = "../web/out"]
#[allow_missing = true]
struct Ui;

const NOT_BUILT_PAGE: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Lumingo</title></head><body><p>The web UI has not been built. Run <code>pnpm --dir apps/web build</code> and rebuild the server.</p></body></html>";

/// Fallback handler: every request that is not an API route.
pub async fn serve_ui(req: Request) -> Response {
    if !matches!(*req.method(), Method::GET | Method::HEAD) {
        return plain(StatusCode::METHOD_NOT_ALLOWED, "method not allowed");
    }
    let Some(path) = clean_path(req.uri().path()) else {
        return plain(StatusCode::BAD_REQUEST, "bad path");
    };

    if Ui::get("index.html").is_none() {
        return html_response(
            StatusCode::SERVICE_UNAVAILABLE,
            NOT_BUILT_PAGE.as_bytes().to_vec(),
            &req,
        );
    }

    for candidate in candidates(&path) {
        if let Some(file) = Ui::get(&candidate) {
            let mime = file.metadata.mimetype().to_owned();
            return file_response(
                StatusCode::OK,
                &candidate,
                &mime,
                file.data.into_owned(),
                &req,
            );
        }
    }
    match Ui::get("404.html") {
        Some(file) => file_response(
            StatusCode::NOT_FOUND,
            "404.html",
            "text/html",
            file.data.into_owned(),
            &req,
        ),
        None => plain(StatusCode::NOT_FOUND, "not found"),
    }
}

/// Strips the leading slash and rejects anything that could leave the UI folder.
fn clean_path(raw: &str) -> Option<String> {
    if raw.contains(['\\', '\0']) {
        return None;
    }
    let trimmed = raw.trim_start_matches('/');
    if trimmed.split('/').any(|segment| segment == "..") {
        return None;
    }
    Some(trimmed.to_owned())
}

/// File names a request path may refer to inside a static export.
fn candidates(path: &str) -> Vec<String> {
    if path.is_empty() {
        return vec!["index.html".to_owned()];
    }
    if let Some(dir) = path.strip_suffix('/') {
        return vec![format!("{dir}/index.html")];
    }
    let last = path.rsplit('/').next().unwrap_or(path);
    if last.contains('.') {
        return vec![path.to_owned()];
    }
    vec![format!("{path}.html"), format!("{path}/index.html")]
}

fn file_response(
    status: StatusCode,
    name: &str,
    mime: &str,
    data: Vec<u8>,
    req: &Request,
) -> Response {
    if mime.starts_with("text/html") {
        return html_response(status, data, req);
    }
    let mut response = Response::new(if *req.method() == Method::HEAD {
        Body::empty()
    } else {
        Body::from(data)
    });
    *response.status_mut() = status;
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(mime) {
        headers.insert(header::CONTENT_TYPE, value);
    }
    let cache = if name.starts_with("_next/static/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response
}

fn html_response(status: StatusCode, data: Vec<u8>, req: &Request) -> Response {
    let csp = html_csp(&data);
    let mut response = Response::new(if *req.method() == Method::HEAD {
        Body::empty()
    } else {
        Body::from(data)
    });
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    if let Ok(value) = HeaderValue::from_str(&csp) {
        headers.insert(header::CONTENT_SECURITY_POLICY, value);
    }
    response
}

fn plain(status: StatusCode, text: &'static str) -> Response {
    let mut response = Response::new(Body::from(text));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
}

/// Content-Security-Policy for one HTML page. A static export contains inline
/// scripts, so each one is allowed by its hash instead of by `'unsafe-inline'`.
/// Styles keep `'unsafe-inline'` because React writes style attributes.
pub fn html_csp(html: &[u8]) -> String {
    let hashes = inline_script_hashes(&String::from_utf8_lossy(html));
    let script_src = if hashes.is_empty() {
        "'self'".to_owned()
    } else {
        format!("'self' {}", hashes.join(" "))
    };
    format!(
        "default-src 'self'; script-src {script_src}; style-src 'self' 'unsafe-inline'; \
         img-src 'self' data:; font-src 'self'; connect-src 'self'; object-src 'none'; \
         base-uri 'none'; form-action 'self'; frame-ancestors 'none'"
    )
}

/// `'sha256-...'` sources for every `<script>` without a `src` attribute.
fn inline_script_hashes(html: &str) -> Vec<String> {
    let mut hashes = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find("<script") {
        let after_tag_name = &rest[start + "<script".len()..];
        let Some(tag_end) = after_tag_name.find('>') else {
            break;
        };
        let attributes = &after_tag_name[..tag_end];
        let body_and_rest = &after_tag_name[tag_end + 1..];
        let Some(close) = body_and_rest.find("</script>") else {
            break;
        };
        let body = &body_and_rest[..close];
        let has_src = attributes
            .split_whitespace()
            .any(|a| a == "src" || a.starts_with("src="));
        if !has_src && !body.is_empty() {
            let digest = Sha256::digest(body.as_bytes());
            hashes.push(format!("'sha256-{}'", STANDARD.encode(digest)));
        }
        rest = &body_and_rest[close + "</script>".len()..];
    }
    hashes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_cover_static_export_layouts() {
        assert_eq!(candidates(""), ["index.html"]);
        assert_eq!(candidates("unit/"), ["unit/index.html"]);
        assert_eq!(candidates("unit"), ["unit.html", "unit/index.html"]);
        assert_eq!(candidates("_next/static/a.js"), ["_next/static/a.js"]);
    }

    #[test]
    fn clean_path_rejects_traversal() {
        assert_eq!(clean_path("/a/b"), Some("a/b".to_owned()));
        assert_eq!(clean_path("/../secret"), None);
        assert_eq!(clean_path("/a/../b"), None);
        assert_eq!(clean_path("/a\\b"), None);
    }

    #[test]
    fn csp_allows_inline_scripts_by_hash_only() {
        let html = r#"<html><script src="/a.js"></script><script>self.x=1</script></html>"#;
        let csp = html_csp(html.as_bytes());
        assert!(csp.contains("'sha256-"));
        assert!(!csp.contains("script-src 'self' 'unsafe-inline'"));
        // The hash of the external script must not appear: only one hash is present.
        assert_eq!(csp.matches("'sha256-").count(), 1);
    }

    #[test]
    fn csp_without_inline_scripts_is_self_only() {
        let csp = html_csp(b"<html><script src=\"/a.js\"></script></html>");
        assert!(csp.contains("script-src 'self';"));
    }
}
