//! Answering a certificate authority's challenges: an HTTP file on a port,
//! a TLS handshake with a special certificate, and (in [`super::cloudflare`])
//! a DNS record. Each lives only while an order is being validated.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::Router;
use rcgen::{CertificateParams, CustomExtension, KeyPair};
use rustls::sign::CertifiedKey;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use crate::tls::{self, Resolver};

/// Listen on `port` on every address, IPv6 and IPv4: a name with an AAAA
/// record is validated over IPv6 first. Fails only if neither could be bound.
async fn bind_both(port: u16) -> Result<Vec<TcpListener>, String> {
    let mut listeners = Vec::new();
    let mut last_error = None;
    for address in [format!("[::]:{port}"), format!("0.0.0.0:{port}")] {
        match TcpListener::bind(&address).await {
            Ok(listener) => listeners.push(listener),
            Err(error) => last_error = Some(error),
        }
    }
    match (listeners.is_empty(), last_error) {
        (true, Some(error)) => Err(format!(
            "port {port} cannot be listened on ({error}); something else uses it, or this needs more rights"
        )),
        _ => Ok(listeners),
    }
}

/// Serves `/.well-known/acme-challenge/<token>` while it lives.
pub struct HttpSolver {
    tokens: Arc<RwLock<HashMap<String, String>>>,
    tasks: Vec<JoinHandle<()>>,
}

impl HttpSolver {
    pub async fn start(port: u16) -> Result<Self, String> {
        let tokens: Arc<RwLock<HashMap<String, String>>> = Arc::default();
        let app = Router::new()
            .route(
                "/.well-known/acme-challenge/{token}",
                get(
                    |State(tokens): State<Arc<RwLock<HashMap<String, String>>>>,
                     Path(token): Path<String>| async move {
                        match tokens.read().unwrap_or_else(|p| p.into_inner()).get(&token) {
                            Some(answer) => (StatusCode::OK, answer.clone()),
                            None => (StatusCode::NOT_FOUND, String::new()),
                        }
                    },
                ),
            )
            .with_state(Arc::clone(&tokens));
        let tasks = bind_both(port)
            .await?
            .into_iter()
            .map(|listener| {
                let app = app.clone();
                tokio::spawn(async move {
                    let _ = axum::serve(listener, app).await;
                })
            })
            .collect();
        Ok(Self { tokens, tasks })
    }

    /// Answer `token` with `key_authorization`.
    pub fn add(&self, token: &str, key_authorization: &str) {
        self.tokens
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(token.to_string(), key_authorization.to_string());
    }
}

impl Drop for HttpSolver {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// The server name a TLS-ALPN-01 challenge for `identifier` is asked with: the
/// name itself, or for an address the name it reverse-maps to (RFC 8738).
pub fn alpn_server_name(identifier: &str) -> String {
    match identifier.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            let [a, b, c, d] = ip.octets();
            format!("{d}.{c}.{b}.{a}.in-addr.arpa")
        }
        Ok(IpAddr::V6(ip)) => {
            let nibbles: String = ip
                .octets()
                .iter()
                .rev()
                .flat_map(|byte| [byte & 0x0f, byte >> 4])
                .map(|n| format!("{n:x}."))
                .collect();
            format!("{nibbles}ip6.arpa")
        }
        Err(_) => identifier.to_ascii_lowercase(),
    }
}

/// The certificate a TLS-ALPN-01 challenge is answered with (RFC 8737): one
/// for the identifier, with the digest of the key authorization in a critical
/// extension.
pub fn challenge_certificate(identifier: &str, digest: &[u8]) -> Result<Arc<CertifiedKey>, String> {
    let mut params = CertificateParams::new(vec![identifier.to_string()])
        .map_err(|e| format!("a challenge certificate for {identifier}: {e}"))?;
    params.custom_extensions = vec![CustomExtension::new_acme_identifier(digest)];
    params.not_before = time::OffsetDateTime::now_utc() - time::Duration::days(1);
    params.not_after = time::OffsetDateTime::now_utc() + time::Duration::days(7);
    let key = KeyPair::generate().map_err(|e| e.to_string())?;
    let certificate = params.self_signed(&key).map_err(|e| e.to_string())?;
    // Made together just now, so not checked: the extension is critical, and
    // a strict parser would refuse the certificate for it.
    tls::certified_key_unchecked(certificate.pem().as_bytes(), key.serialize_pem().as_bytes())
        .map_err(|e| e.to_string())
}

/// Answers TLS-ALPN-01 on the panel's own HTTPS port when that is the one the
/// authority will ask, else on a port of its own for the while.
pub struct AlpnSolver {
    resolver: Arc<Resolver>,
    names: std::sync::Mutex<Vec<String>>,
    tasks: Vec<JoinHandle<()>>,
}

impl AlpnSolver {
    /// `panel` is the panel's own certificate resolver and the port it
    /// listens on, when it serves HTTPS.
    pub async fn start(panel: Option<(Arc<Resolver>, u16)>, port: u16) -> Result<Self, String> {
        if let Some((resolver, panel_port)) = panel {
            if panel_port == port {
                return Ok(Self {
                    resolver,
                    names: Default::default(),
                    tasks: Vec::new(),
                });
            }
        }
        let resolver = Arc::new(Resolver::default());
        let acceptor = tls::acceptor(Arc::clone(&resolver)).map_err(|e| e.to_string())?;
        let tasks = bind_both(port)
            .await?
            .into_iter()
            .map(|listener| {
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    while let Ok((stream, _)) = listener.accept().await {
                        let acceptor = acceptor.clone();
                        // A challenge is only the handshake.
                        tokio::spawn(async move {
                            let _ = tokio::time::timeout(
                                Duration::from_secs(10),
                                acceptor.accept(stream),
                            )
                            .await;
                        });
                    }
                })
            })
            .collect();
        Ok(Self {
            resolver,
            names: Default::default(),
            tasks,
        })
    }

    /// Answer the challenge for `identifier`, whose key authorization has
    /// this `digest`.
    pub fn add(&self, identifier: &str, digest: &[u8]) -> Result<(), String> {
        let name = alpn_server_name(identifier);
        self.resolver
            .add_challenge(&name, challenge_certificate(identifier, digest)?);
        self.names
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(name);
        Ok(())
    }
}

impl Drop for AlpnSolver {
    fn drop(&mut self) {
        for name in self.names.lock().unwrap_or_else(|p| p.into_inner()).iter() {
            self.resolver.remove_challenge(name);
        }
        for task in &self.tasks {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_asked_for_by_the_name_it_reverse_maps_to() {
        assert_eq!(alpn_server_name("Example.COM"), "example.com");
        assert_eq!(alpn_server_name("203.0.113.7"), "7.113.0.203.in-addr.arpa");
        assert_eq!(
            alpn_server_name("2001:db8::567:89ab"),
            "b.a.9.8.7.6.5.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.8.b.d.0.1.0.0.2.ip6.arpa"
        );
    }

    #[test]
    fn a_challenge_certificate_is_made_for_a_name_and_for_an_address() {
        let digest = [7u8; 32];
        for identifier in ["panel.example.com", "203.0.113.7", "2001:db8::1"] {
            let key = challenge_certificate(identifier, &digest).unwrap();
            let (_, cert) = x509_parser::parse_x509_certificate(key.cert[0].as_ref()).unwrap();
            // The acmeIdentifier extension (1.3.6.1.5.5.7.1.31), critical,
            // holding an OCTET STRING of the digest.
            let extension = cert
                .extensions()
                .iter()
                .find(|e| e.oid.to_id_string() == "1.3.6.1.5.5.7.1.31")
                .unwrap_or_else(|| panic!("no acmeIdentifier for {identifier}"));
            assert!(extension.critical);
            assert_eq!(&extension.value[extension.value.len() - 32..], &digest);
            let names = cert.subject_alternative_name().unwrap().unwrap();
            assert_eq!(names.value.general_names.len(), 1, "{identifier}");
        }
    }

    #[tokio::test]
    async fn the_http_solver_answers_a_token_it_was_given_and_only_that_and_stops_with_its_owner() {
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let solver = HttpSolver::start(port).await.unwrap();
        solver.add("tok-1", "tok-1.thumb");
        let url =
            |token: &str| format!("http://127.0.0.1:{port}/.well-known/acme-challenge/{token}");
        let asked = crate::outbound::send(crate::outbound::Request::get(&url("tok-1")))
            .await
            .unwrap();
        assert_eq!((asked.status, asked.text()), (200, "tok-1.thumb".into()));
        let other = crate::outbound::send(crate::outbound::Request::get(&url("tok-2")))
            .await
            .unwrap();
        assert_eq!(other.status, 404);
        let elsewhere = crate::outbound::send(crate::outbound::Request::get(&format!(
            "http://127.0.0.1:{port}/index.html"
        )))
        .await
        .unwrap();
        assert_eq!(elsewhere.status, 404);

        drop(solver);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            crate::outbound::send(crate::outbound::Request::get(&url("tok-1")))
                .await
                .is_err(),
            "the port is closed again"
        );
    }

    #[tokio::test]
    async fn a_port_in_use_is_said_so() {
        let held = TcpListener::bind("0.0.0.0:0").await.unwrap();
        let port = held.local_addr().unwrap().port();
        let error = HttpSolver::start(port).await.err().expect("refused");
        assert!(error.contains(&port.to_string()), "{error}");
    }
}
