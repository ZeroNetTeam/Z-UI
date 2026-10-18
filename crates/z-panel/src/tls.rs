//! HTTPS for the panel itself, from a certificate and key on disk.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use axum::extract::ConnectInfo;
use axum::Router;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tower::ServiceExt;

/// A TLS acceptor for `cert` (the chain) and `key`, both PEM.
pub fn acceptor(cert: &Path, key: &Path) -> anyhow::Result<TlsAcceptor> {
    let chain = CertificateDer::pem_file_iter(cert)
        .with_context(|| format!("reading {}", cert.display()))?
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("parsing {}", cert.display()))?;
    anyhow::ensure!(!chain.is_empty(), "{} holds no certificate", cert.display());
    let key = PrivateKeyDer::from_pem_file(key)
        .with_context(|| format!("reading the key {}", key.display()))?;
    // The build carries two crypto providers (zray-core uses both), so the
    // one used here is named rather than guessed.
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .context("TLS versions")?
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .context("the certificate and key do not match")?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
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
