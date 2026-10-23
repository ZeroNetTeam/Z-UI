//! HTTPS for the panel itself. The certificate lives in a [`Resolver`] that
//! the server asks on every handshake, so a renewed one is in force for the
//! next connection without a restart, and a TLS-ALPN-01 challenge can be
//! answered on the same port.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

use anyhow::Context;
use axum::extract::ConnectInfo;
use axum::Router;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tower::ServiceExt;

/// The ALPN name of the TLS-ALPN-01 challenge (RFC 8737).
pub const ACME_ALPN: &[u8] = b"acme-tls/1";

/// The certificate chain and key from PEM text, checked to belong together.
pub fn certified_key(cert_pem: &[u8], key_pem: &[u8]) -> anyhow::Result<Arc<CertifiedKey>> {
    let chain = CertificateDer::pem_slice_iter(cert_pem)
        .collect::<Result<Vec<_>, _>>()
        .context("the certificate is not PEM")?;
    anyhow::ensure!(!chain.is_empty(), "no certificate in the PEM text");
    let key = PrivateKeyDer::from_pem_slice(key_pem).context("the private key is not PEM")?;
    // The build carries two crypto providers (zray-core uses both), so the
    // one used here is named rather than guessed.
    let provider = rustls::crypto::ring::default_provider();
    let certified = CertifiedKey::from_der(chain, key, &provider)
        .context("the private key does not belong to this certificate")?;
    Ok(Arc::new(certified))
}

/// A certificate chain and key from PEM text, taken as they are: not checked
/// to belong together. For one just made together, such as an ACME challenge
/// certificate, whose critical extension a strict parser refuses to read.
pub fn certified_key_unchecked(
    cert_pem: &[u8],
    key_pem: &[u8],
) -> anyhow::Result<Arc<CertifiedKey>> {
    let chain = CertificateDer::pem_slice_iter(cert_pem)
        .collect::<Result<Vec<_>, _>>()
        .context("the certificate is not PEM")?;
    anyhow::ensure!(!chain.is_empty(), "no certificate in the PEM text");
    let key = PrivateKeyDer::from_pem_slice(key_pem).context("the private key is not PEM")?;
    let provider = rustls::crypto::ring::default_provider();
    let signing_key = provider
        .key_provider
        .load_private_key(key)
        .context("the private key cannot be used")?;
    Ok(Arc::new(CertifiedKey::new(chain, signing_key)))
}

/// The certificate `id` of the store, ready to present.
pub async fn stored_certificate(db: &z_db::Database, id: i64) -> anyhow::Result<Arc<CertifiedKey>> {
    let found = db
        .certificate(id)
        .await?
        .with_context(|| format!("there is no certificate {id}"))?;
    certified_key(found.cert_pem.as_bytes(), found.key_pem.as_bytes())
        .with_context(|| format!("certificate {id} ({}) cannot be served", found.name))
}

/// Which certificate a handshake gets.
#[derive(Default)]
pub struct Resolver {
    current: RwLock<Option<Arc<CertifiedKey>>>,
    /// Certificates that answer TLS-ALPN-01 challenges, by the name asked for.
    challenges: RwLock<HashMap<String, Arc<CertifiedKey>>>,
}

impl std::fmt::Debug for Resolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resolver").finish_non_exhaustive()
    }
}

impl Resolver {
    pub fn new(key: Arc<CertifiedKey>) -> Arc<Self> {
        let resolver = Self::default();
        resolver.set(key);
        Arc::new(resolver)
    }

    /// From PEM files on disk.
    pub fn from_files(cert: &Path, key: &Path) -> anyhow::Result<Arc<Self>> {
        Ok(Self::new(read_files(cert, key)?))
    }

    /// From PEM text, as the certificate store keeps it.
    #[cfg(test)]
    pub fn from_pem(cert_pem: &str, key_pem: &str) -> anyhow::Result<Arc<Self>> {
        Ok(Self::new(certified_key(
            cert_pem.as_bytes(),
            key_pem.as_bytes(),
        )?))
    }

    /// Present `key` from the next handshake on.
    pub fn set(&self, key: Arc<CertifiedKey>) {
        *self.current.write().unwrap_or_else(|p| p.into_inner()) = Some(key);
    }

    /// The leaf certificate being presented, DER.
    #[cfg(test)]
    pub fn leaf(&self) -> Option<Vec<u8>> {
        self.current
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .and_then(|key| key.cert.first().map(|c| c.as_ref().to_vec()))
    }

    /// Answer TLS-ALPN-01 for `name` with `key` until [`Self::remove_challenge`].
    pub fn add_challenge(&self, name: &str, key: Arc<CertifiedKey>) {
        self.challenges
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(name.to_ascii_lowercase(), key);
    }

    pub fn remove_challenge(&self, name: &str) {
        self.challenges
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&name.to_ascii_lowercase());
    }
}

impl ResolvesServerCert for Resolver {
    fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let wants_challenge = hello
            .alpn()
            .is_some_and(|mut protocols| protocols.any(|p| p == ACME_ALPN));
        if wants_challenge {
            // Only a challenge in progress is answered; nothing else gets
            // the special certificate, and the challenge gets nothing else.
            let name = hello.server_name()?.to_ascii_lowercase();
            return self
                .challenges
                .read()
                .unwrap_or_else(|p| p.into_inner())
                .get(&name)
                .cloned();
        }
        self.current
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
}

fn read_files(cert: &Path, key: &Path) -> anyhow::Result<Arc<CertifiedKey>> {
    let cert_pem = std::fs::read(cert).with_context(|| format!("reading {}", cert.display()))?;
    let key_pem =
        std::fs::read(key).with_context(|| format!("reading the key {}", key.display()))?;
    certified_key(&cert_pem, &key_pem).with_context(|| format!("with {}", cert.display()))
}

/// How often files that were given as the certificate are looked at again,
/// so one a tool like certbot renews is picked up.
const FILE_WATCH: Duration = Duration::from_secs(300);

/// Read `cert` and `key` again whenever either changes on disk.
pub fn watch_files(resolver: Arc<Resolver>, cert: PathBuf, key: PathBuf) {
    fn stamp(cert: &Path, key: &Path) -> Option<(SystemTime, SystemTime)> {
        Some((
            std::fs::metadata(cert).ok()?.modified().ok()?,
            std::fs::metadata(key).ok()?.modified().ok()?,
        ))
    }
    tokio::spawn(async move {
        let mut seen = stamp(&cert, &key);
        loop {
            tokio::time::sleep(FILE_WATCH).await;
            let now = stamp(&cert, &key);
            if now == seen {
                continue;
            }
            match read_files(&cert, &key) {
                Ok(renewed) => {
                    resolver.set(renewed);
                    seen = now;
                    tracing::info!(cert = %cert.display(), "the panel's certificate files changed and were read again");
                }
                // A renewal writes two files: wait for the second.
                Err(error) => {
                    tracing::warn!(%error, "the panel's certificate files could not be read; the old certificate stays")
                }
            }
        }
    });
}

/// Accepts whatever certificate a server shows. Only for asking this panel,
/// on this machine, whether it answers (the health check) and for the test
/// authority: it proves nothing about who answers.
#[derive(Debug)]
struct AcceptAll;

impl rustls::client::danger::ServerCertVerifier for AcceptAll {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &rustls_pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls_pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// A client config that trusts any certificate, offering `alpn`.
pub fn insecure_client(alpn: &[&[u8]]) -> Arc<rustls::ClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("the default TLS versions")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAll))
        .with_no_client_auth();
    config.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
    Arc::new(config)
}

/// A TLS acceptor that asks `resolver` for the certificate.
pub fn acceptor(resolver: Arc<Resolver>) -> anyhow::Result<TlsAcceptor> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .context("TLS versions")?
        .with_no_client_auth()
        .with_cert_resolver(resolver);
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec(), ACME_ALPN.to_vec()];
    Ok(TlsAcceptor::from(Arc::new(config)))
}

/// Serve `app` over TLS until `shutdown` resolves.
pub async fn serve(
    listener: TcpListener,
    acceptor: TlsAcceptor,
    app: Router,
    shutdown: impl std::future::Future<Output = ()>,
) {
    tokio::pin!(shutdown);
    loop {
        let (stream, peer) = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(accepted) => accepted,
                Err(error) => {
                    tracing::warn!(%error, "accept failed");
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    continue;
                }
            },
            () = &mut shutdown => return,
        };
        let (acceptor, app) = (acceptor.clone(), app.clone());
        tokio::spawn(async move {
            let Ok(Ok(stream)) =
                tokio::time::timeout(std::time::Duration::from_secs(10), acceptor.accept(stream))
                    .await
            else {
                return;
            };
            // A challenge connection ends with its handshake: it carries no
            // HTTP, and must not reach the panel.
            if stream.get_ref().1.alpn_protocol() == Some(ACME_ALPN) {
                return;
            }
            let service = hyper::service::service_fn(move |mut request: hyper::Request<_>| {
                request
                    .extensions_mut()
                    .insert(ConnectInfo::<SocketAddr>(peer));
                app.clone().oneshot(request.map(axum::body::Body::new))
            });
            let _ = auto::Builder::new(TokioExecutor::new())
                .serve_connection_with_upgrades(TokioIo::new(stream), service)
                .await;
        });
    }
}

#[cfg(test)]
pub mod testing {
    //! Certificates made on the spot, and a client that trusts them.

    use super::*;

    /// A self-signed `(cert_pem, key_pem)` for `names`, valid for `days`.
    pub fn self_signed(names: &[&str], days: i64) -> (String, String) {
        let mut params =
            rcgen::CertificateParams::new(names.iter().map(|n| n.to_string()).collect::<Vec<_>>())
                .expect("names");
        params.not_before = rcgen::date_time_ymd(2020, 1, 1);
        let after = time::OffsetDateTime::now_utc() + time::Duration::days(days);
        params.not_after = after;
        let key = rcgen::KeyPair::generate().expect("key");
        let cert = params.self_signed(&key).expect("cert");
        (cert.pem(), key.serialize_pem())
    }

    /// A client config that trusts exactly `cert_pem`.
    pub fn client_trusting(cert_pem: &str) -> Arc<rustls::ClientConfig> {
        let mut roots = rustls::RootCertStore::empty();
        for cert in CertificateDer::pem_slice_iter(cert_pem.as_bytes()) {
            roots.add(cert.expect("pem")).expect("root");
        }
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        Arc::new(
            rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .expect("versions")
                .with_root_certificates(roots)
                .with_no_client_auth(),
        )
    }

    /// The DER of the leaf a server presents when asked for `name`.
    pub async fn leaf_seen(addr: SocketAddr, name: &str, trusting: &str) -> Vec<u8> {
        let connector = tokio_rustls::TlsConnector::from(client_trusting(trusting));
        let tcp = tokio::net::TcpStream::connect(addr).await.expect("connect");
        let stream = connector
            .connect(
                rustls_pki_types::ServerName::try_from(name.to_string()).expect("name"),
                tcp,
            )
            .await
            .expect("handshake");
        stream.get_ref().1.peer_certificates().expect("certs")[0]
            .as_ref()
            .to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    #[test]
    fn a_key_that_is_not_the_certificates_is_refused() {
        let (cert, _) = self_signed(&["a.test"], 30);
        let (_, other_key) = self_signed(&["b.test"], 30);
        assert!(certified_key(cert.as_bytes(), other_key.as_bytes()).is_err());
        assert!(certified_key(b"nothing", other_key.as_bytes()).is_err());
    }

    #[tokio::test]
    async fn a_renewed_certificate_is_presented_from_the_next_handshake_without_a_restart() {
        let (old_cert, old_key) = self_signed(&["panel.test"], 30);
        let resolver = Resolver::from_pem(&old_cert, &old_key).unwrap();
        let acceptor = acceptor(resolver.clone()).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new().route("/", axum::routing::get(|| async { "ok" }));
        tokio::spawn(serve(listener, acceptor, app, std::future::pending()));

        let first = leaf_seen(addr, "panel.test", &old_cert).await;
        assert_eq!(Some(first.clone()), resolver.leaf());

        let (new_cert, new_key) = self_signed(&["panel.test"], 90);
        resolver.set(certified_key(new_cert.as_bytes(), new_key.as_bytes()).unwrap());
        let second = leaf_seen(addr, "panel.test", &new_cert).await;
        assert_ne!(
            first, second,
            "the new certificate is what a new client sees"
        );
    }

    #[tokio::test]
    async fn only_a_challenge_in_progress_is_answered_on_the_acme_protocol() {
        let (cert, key) = self_signed(&["panel.test"], 30);
        let resolver = Resolver::from_pem(&cert, &key).unwrap();
        let acceptor = acceptor(resolver.clone()).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new().route("/", axum::routing::get(|| async { "ok" }));
        tokio::spawn(serve(listener, acceptor, app, std::future::pending()));

        // A client that speaks only acme-tls/1 gets nothing without a challenge.
        let ask = |name: &'static str, trusting: String| async move {
            let mut config = (*client_trusting(&trusting)).clone();
            config.alpn_protocols = vec![ACME_ALPN.to_vec()];
            let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
            let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
            connector
                .connect(
                    rustls_pki_types::ServerName::try_from(name.to_string()).unwrap(),
                    tcp,
                )
                .await
                .map(|stream| {
                    stream.get_ref().1.peer_certificates().unwrap()[0]
                        .as_ref()
                        .to_vec()
                })
        };
        assert!(ask("panel.test", cert.clone()).await.is_err());

        let (challenge_cert, challenge_key) = self_signed(&["panel.test"], 1);
        resolver.add_challenge(
            "Panel.Test",
            certified_key(challenge_cert.as_bytes(), challenge_key.as_bytes()).unwrap(),
        );
        let seen = ask("panel.test", challenge_cert.clone()).await.unwrap();
        assert_ne!(
            Some(seen),
            resolver.leaf(),
            "the challenge has a certificate of its own"
        );
        assert!(
            ask("other.test", challenge_cert).await.is_err(),
            "another name has none"
        );

        resolver.remove_challenge("panel.test");
        assert!(
            ask("panel.test", cert).await.is_err(),
            "and it ends with the challenge"
        );
    }
}
