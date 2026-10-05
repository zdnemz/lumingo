//! The one HTTP client factory (PRD NFR-S1, `context_pack.md` section 10).
//!
//! Every outbound request in this crate is built through `GuardedClient`, which
//! checks the URL against the allowlist and the HTTPS rule before a socket is
//! opened. Redirects are followed only to allowlisted hosts, so a provider cannot
//! bounce a request to another host. System proxies are not used: they would be a
//! second destination outside the allowlist and would make behaviour depend on the
//! machine.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use reqwest::redirect::Policy;
use reqwest::{Client, Url};

use crate::error::LlmError;

const USER_AGENT: &str = concat!("lumingo-llm-client/", env!("CARGO_PKG_VERSION"));
const MAX_REDIRECTS: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct HostPort {
    host: String,
    port: u16,
}

impl fmt::Display for HostPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

fn host_port(url: &Url) -> Option<HostPort> {
    let host = url.host()?.to_string().to_ascii_lowercase();
    let port = url.port_or_known_default()?;
    Some(HostPort { host, port })
}

/// `localhost` and loopback addresses (`127.0.0.0/8`, `::1`).
pub fn is_loopback_url(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

#[derive(Debug, Default)]
struct Entries {
    active: Option<HostPort>,
    setup: BTreeSet<HostPort>,
}

/// Marker error carried through `reqwest` when a redirect target is refused.
#[derive(Debug)]
struct RedirectRefused;

impl fmt::Display for RedirectRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("redirect to a host that is not on the allowlist")
    }
}

impl std::error::Error for RedirectRefused {}

#[derive(Debug, Default)]
struct AllowPolicy {
    entries: RwLock<Entries>,
}

impl AllowPolicy {
    fn check(&self, url: &Url) -> Result<(), LlmError> {
        let Some(target) = host_port(url) else {
            return Err(LlmError::InvalidRequest("the URL has no host".to_owned()));
        };
        match url.scheme() {
            "https" => {}
            "http" if is_loopback_url(url) => {}
            "http" => return Err(LlmError::InsecureScheme { host: target.host }),
            _ => {
                return Err(LlmError::InvalidRequest(
                    "only http and https URLs are supported".to_owned(),
                ));
            }
        }
        let entries = self.entries.read().unwrap_or_else(PoisonError::into_inner);
        let allowed = entries.active.as_ref() == Some(&target) || entries.setup.contains(&target);
        if allowed {
            Ok(())
        } else {
            Err(LlmError::HostNotAllowed {
                host: target.to_string(),
            })
        }
    }
}

/// Builds clients that refuse every host except the active provider's and, while
/// a setup guard is alive, the model hosts named for setup.
#[derive(Clone)]
pub struct HttpClientFactory {
    policy: Arc<AllowPolicy>,
    client: Client,
}

impl fmt::Debug for HttpClientFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpClientFactory").finish_non_exhaustive()
    }
}

impl HttpClientFactory {
    /// `base_url` is the active provider's base URL. It must be HTTPS, or HTTP on
    /// a loopback host.
    pub fn new(base_url: &Url, connect_timeout: Duration) -> Result<Self, LlmError> {
        let policy = Arc::new(AllowPolicy::default());
        let factory_policy = Arc::clone(&policy);
        let redirect = Policy::custom(move |attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                return attempt.error("too many redirects");
            }
            if factory_policy.check(attempt.url()).is_ok() {
                attempt.follow()
            } else {
                attempt.error(RedirectRefused)
            }
        });
        let client = Client::builder()
            .connect_timeout(connect_timeout)
            .redirect(redirect)
            .no_proxy()
            .user_agent(USER_AGENT)
            .min_tls_version(reqwest::tls::Version::TLS_1_2)
            .build()
            .map_err(|_| {
                LlmError::InvalidRequest("the HTTP client could not be created".to_owned())
            })?;
        let factory = Self { policy, client };
        factory.set_active_provider(base_url)?;
        Ok(factory)
    }

    /// Replaces the active provider's host, for example when the learner switches profile.
    pub fn set_active_provider(&self, base_url: &Url) -> Result<(), LlmError> {
        if base_url.scheme() == "http" && !is_loopback_url(base_url) {
            return Err(LlmError::InsecureScheme {
                host: host_port(base_url).map(|h| h.host).unwrap_or_default(),
            });
        }
        let entry = host_port(base_url)
            .ok_or_else(|| LlmError::InvalidRequest("the base URL has no host".to_owned()))?;
        let mut entries = self
            .policy
            .entries
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        entries.active = Some(entry);
        Ok(())
    }

    /// Allows model download hosts until the returned guard is dropped. Each entry is
    /// a URL such as `https://huggingface.co`; only its host and port are used.
    pub fn allow_setup_hosts(&self, hosts: &[Url]) -> Result<SetupHosts, LlmError> {
        let mut added = Vec::with_capacity(hosts.len());
        for url in hosts {
            if url.scheme() == "http" && !is_loopback_url(url) {
                return Err(LlmError::InsecureScheme {
                    host: host_port(url).map(|h| h.host).unwrap_or_default(),
                });
            }
            added.push(
                host_port(url).ok_or_else(|| {
                    LlmError::InvalidRequest("the host URL has no host".to_owned())
                })?,
            );
        }
        let mut entries = self
            .policy
            .entries
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        let fresh: Vec<HostPort> = added
            .into_iter()
            .filter(|h| entries.setup.insert(h.clone()))
            .collect();
        Ok(SetupHosts {
            policy: Arc::clone(&self.policy),
            hosts: fresh,
        })
    }

    pub fn client(&self) -> GuardedClient {
        GuardedClient {
            policy: Arc::clone(&self.policy),
            client: self.client.clone(),
        }
    }
}

/// Keeps setup hosts allowed. Dropping it removes the hosts it added.
#[must_use = "the hosts are allowed only while this guard is alive"]
pub struct SetupHosts {
    policy: Arc<AllowPolicy>,
    hosts: Vec<HostPort>,
}

impl Drop for SetupHosts {
    fn drop(&mut self) {
        let mut entries = self
            .policy
            .entries
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        for host in &self.hosts {
            entries.setup.remove(host);
        }
    }
}

/// A client that cannot reach a host the factory did not allow. The inner
/// `reqwest::Client` is never exposed.
#[derive(Clone)]
pub struct GuardedClient {
    policy: Arc<AllowPolicy>,
    client: Client,
}

impl fmt::Debug for GuardedClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GuardedClient").finish_non_exhaustive()
    }
}

impl GuardedClient {
    /// Checks the allowlist and the HTTPS rule without sending anything.
    pub fn check(&self, url: &Url) -> Result<(), LlmError> {
        self.policy.check(url)
    }

    pub fn post(&self, url: &Url) -> Result<reqwest::RequestBuilder, LlmError> {
        self.policy.check(url)?;
        Ok(self.client.post(url.clone()))
    }

    pub fn get(&self, url: &Url) -> Result<reqwest::RequestBuilder, LlmError> {
        self.policy.check(url)?;
        Ok(self.client.get(url.clone()))
    }
}

/// Maps a `reqwest` error to a typed error without keeping the original, which
/// can hold the request URL.
pub(crate) fn map_reqwest_error(error: &reqwest::Error) -> LlmError {
    use crate::error::{TimeoutKind, TransportKind};

    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(current) = source {
        if current.downcast_ref::<RedirectRefused>().is_some() {
            return LlmError::HostNotAllowed {
                host: "redirect target".to_owned(),
            };
        }
        source = current.source();
    }
    if error.is_timeout() {
        return if error.is_connect() {
            LlmError::Timeout(TimeoutKind::Connect)
        } else {
            LlmError::Transport(TransportKind::Timeout)
        };
    }
    if error.is_connect() {
        return LlmError::Transport(TransportKind::Connect);
    }
    if error.is_body() || error.is_decode() {
        return LlmError::Transport(TransportKind::Body);
    }
    if error.is_builder() {
        return LlmError::InvalidRequest("the request could not be built".to_owned());
    }
    LlmError::Transport(TransportKind::Other)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).expect("test URL")
    }

    fn factory(base: &str) -> HttpClientFactory {
        HttpClientFactory::new(&url(base), Duration::from_secs(5)).expect("factory")
    }

    #[test]
    fn only_the_active_host_passes() {
        let guarded = factory("https://api.example.com/v1").client();
        assert!(
            guarded
                .check(&url("https://api.example.com/v1/chat/completions"))
                .is_ok()
        );
        assert!(guarded.check(&url("https://API.EXAMPLE.COM/other")).is_ok());
        for other in [
            "https://evil.example.net/",
            "https://api.example.com:8443/",
            "https://sub.api.example.com/",
        ] {
            assert!(
                matches!(
                    guarded.check(&url(other)),
                    Err(LlmError::HostNotAllowed { .. })
                ),
                "{other}"
            );
        }
    }

    #[test]
    fn plain_http_is_refused_unless_loopback() {
        assert!(matches!(
            HttpClientFactory::new(&url("http://api.example.com/v1"), Duration::from_secs(1)),
            Err(LlmError::InsecureScheme { .. })
        ));
        for base in [
            "http://localhost:8080/v1",
            "http://127.0.0.1:9/v1",
            "http://[::1]:8080/v1",
        ] {
            assert!(
                HttpClientFactory::new(&url(base), Duration::from_secs(1)).is_ok(),
                "{base}"
            );
        }
        let guarded = factory("https://api.example.com").client();
        assert!(matches!(
            guarded.check(&url("http://api.example.com/")),
            Err(LlmError::InsecureScheme { .. })
        ));
    }

    #[test]
    fn loopback_http_still_needs_to_be_the_active_host() {
        let guarded = factory("http://127.0.0.1:8080/v1").client();
        assert!(guarded.check(&url("http://127.0.0.1:8080/v1/x")).is_ok());
        assert!(matches!(
            guarded.check(&url("http://127.0.0.1:8081/")),
            Err(LlmError::HostNotAllowed { .. })
        ));
        assert!(matches!(
            guarded.check(&url("http://localhost:8080/")),
            Err(LlmError::HostNotAllowed { .. })
        ));
    }

    #[test]
    fn setup_hosts_are_allowed_only_while_the_guard_lives() {
        let factory = factory("https://api.example.com");
        let guarded = factory.client();
        let model_host = url("https://huggingface.co/");
        assert!(guarded.check(&model_host).is_err());
        let guard = factory
            .allow_setup_hosts(std::slice::from_ref(&model_host))
            .expect("allow");
        assert!(guarded.check(&model_host).is_ok());
        drop(guard);
        assert!(guarded.check(&model_host).is_err());
    }

    #[test]
    fn setup_hosts_must_be_https_or_loopback() {
        let factory = factory("https://api.example.com");
        assert!(
            factory
                .allow_setup_hosts(&[url("http://models.example.com/")])
                .is_err()
        );
    }

    #[test]
    fn switching_provider_replaces_the_active_host() {
        let factory = factory("https://one.example.com");
        factory
            .set_active_provider(&url("https://two.example.com"))
            .expect("switch");
        let guarded = factory.client();
        assert!(guarded.check(&url("https://one.example.com/")).is_err());
        assert!(guarded.check(&url("https://two.example.com/")).is_ok());
    }

    #[test]
    fn other_schemes_are_refused() {
        let guarded = factory("https://api.example.com").client();
        assert!(matches!(
            guarded.check(&url("ftp://api.example.com/")),
            Err(LlmError::InvalidRequest(_))
        ));
    }
}
