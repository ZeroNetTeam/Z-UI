//! Outgoing HTTP for the bot, Discord and webhooks: a POST with a body and
//! headers, optionally through a proxy.
//!
//! The panel often runs where Telegram is filtered, so a proxy is part of the
//! settings: `http://host:port` (CONNECT) or `socks5://host:port`, with
//! `user:pass@` when it asks for one. Names are given to the proxy, not
//! resolved here, so a filtered resolver does not matter. The request itself
//! is made by `zero_net::fetch`, which bounds how much it reads and how long
//! it waits.

use std::time::Duration;

use base64::Engine;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use zero_net::fetch::{post_with_headers, send_over, FetchError, FetchLimits};

/// What a request can come to.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HttpError {
    /// The server answered with this status.
    #[error("HTTP {0}")]
    Status(u16),
    /// It could not be reached, or did not make sense.
    #[error("{0}")]
    Failed(String),
}

impl From<FetchError> for HttpError {
    fn from(error: FetchError) -> Self {
        match error {
            FetchError::Status(code) => HttpError::Status(code),
            other => HttpError::Failed(other.to_string()),
        }
    }
}

/// A proxy to go through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proxy {
    pub kind: ProxyKind,
    /// `host:port`.
    pub address: String,
    pub credentials: Option<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyKind {
    Http,
    Socks5,
}

impl Proxy {
    /// Read `http://[user:pass@]host:port` or `socks5://…` (`socks5h` and
    /// `socks` are the same: the proxy resolves names). Empty is no proxy.
    pub fn parse(text: &str) -> Result<Option<Proxy>, String> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(None);
        }
        let url = url::Url::parse(text).map_err(|error| format!("not a proxy address: {error}"))?;
        let kind = match url.scheme() {
            "http" => ProxyKind::Http,
            "socks5" | "socks5h" | "socks" => ProxyKind::Socks5,
            other => return Err(format!("a proxy is http:// or socks5://, not {other}://")),
        };
        let host = url.host_str().ok_or("the proxy has no host")?;
        let port = url
            .port()
            .unwrap_or(if kind == ProxyKind::Http { 8080 } else { 1080 });
        let credentials = (!url.username().is_empty()).then(|| {
            (
                percent_encoding::percent_decode_str(url.username())
                    .decode_utf8_lossy()
                    .into_owned(),
                percent_encoding::percent_decode_str(url.password().unwrap_or(""))
                    .decode_utf8_lossy()
                    .into_owned(),
            )
        });
        let address = authority(host, port);
        Ok(Some(Proxy {
            kind,
            address,
            credentials,
        }))
    }
}

/// `host:port`, with an IPv6 address in brackets (`url` hands one back
/// bracketed already).
fn authority(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// How much of a reply is read.
const MAX_REPLY: usize = 1024 * 1024;

fn limits(timeout: Duration) -> FetchLimits {
    FetchLimits {
        max_bytes: MAX_REPLY,
        timeout,
        max_redirects: 0,
    }
}

fn failed(what: impl std::fmt::Display) -> HttpError {
    HttpError::Failed(what.to_string())
}

/// POST `body` to `url` and return the reply's body.
///
/// The request itself lives on the heap: the machinery under it is tens of
/// kilobytes of future, and a caller with many places that send (the bot has
/// dozens) would otherwise carry one copy for each of them in its frame.
pub async fn post(
    url: &str,
    content_type: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    proxy: Option<&Proxy>,
    timeout: Duration,
) -> Result<Vec<u8>, HttpError> {
    Box::pin(post_now(url, content_type, headers, body, proxy, timeout)).await
}

async fn post_now(
    url: &str,
    content_type: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    proxy: Option<&Proxy>,
    timeout: Duration,
) -> Result<Vec<u8>, HttpError> {
    let Some(proxy) = proxy else {
        return Ok(post_with_headers(url, content_type, headers, body, &limits(timeout)).await?);
    };
    tokio::time::timeout(
        timeout,
        through(url, content_type, headers, body, proxy, timeout),
    )
    .await
    .map_err(|_| failed(format!("timed out after {timeout:?}")))?
}

async fn through(
    url: &str,
    content_type: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    proxy: &Proxy,
    timeout: Duration,
) -> Result<Vec<u8>, HttpError> {
    let target = url::Url::parse(url).map_err(|error| failed(format!("bad URL: {error}")))?;
    let tls = match target.scheme() {
        "https" => true,
        "http" => false,
        other => return Err(failed(format!("unsupported URL scheme {other}"))),
    };
    let host = target
        .host_str()
        .ok_or_else(|| failed("the URL has no host"))?
        .to_string();
    let port = target
        .port_or_known_default()
        .ok_or_else(|| failed("the URL has no port"))?;

    let mut stream = TcpStream::connect(&proxy.address).await.map_err(|error| {
        failed(format!(
            "connecting to the proxy {}: {error}",
            proxy.address
        ))
    })?;
    match proxy.kind {
        ProxyKind::Http => connect_http(&mut stream, &host, port, proxy).await?,
        ProxyKind::Socks5 => connect_socks5(&mut stream, &host, port, proxy).await?,
    }
    if tls {
        let connector = tokio_rustls::TlsConnector::from(zero_net::fetch::tls_config());
        let name = rustls_pki_types::ServerName::try_from(host.clone())
            .map_err(|error| failed(format!("bad host name: {error}")))?;
        let stream = connector
            .connect(name, stream)
            .await
            .map_err(|error| failed(format!("TLS to {host}: {error}")))?;
        Ok(send_over(
            stream,
            "POST",
            url,
            content_type,
            headers,
            body,
            &limits(timeout),
        )
        .await?)
    } else {
        Ok(send_over(
            stream,
            "POST",
            url,
            content_type,
            headers,
            body,
            &limits(timeout),
        )
        .await?)
    }
}

/// Ask an HTTP proxy for a tunnel to `host:port`.
pub(crate) async fn connect_http(
    stream: &mut TcpStream,
    host: &str,
    port: u16,
    proxy: &Proxy,
) -> Result<(), HttpError> {
    let authority = authority(host, port);
    let mut request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n");
    if let Some((user, password)) = &proxy.credentials {
        let token = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"));
        request.push_str(&format!("Proxy-Authorization: Basic {token}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await.map_err(failed)?;
    // The reply ends at a blank line; read byte by byte so none of what comes
    // after it (the tunnel's own bytes) is taken.
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > 8192 {
            return Err(failed("the proxy's answer is too long"));
        }
        if stream.read(&mut byte).await.map_err(failed)? == 0 {
            return Err(failed("the proxy closed the connection"));
        }
        head.push(byte[0]);
    }
    let line = String::from_utf8_lossy(&head);
    let status = line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or(0);
    match status {
        200 => Ok(()),
        407 => Err(failed("the proxy wants a username and password")),
        other => Err(failed(format!(
            "the proxy refused the tunnel (HTTP {other})"
        ))),
    }
}

/// Ask a SOCKS5 proxy for a connection to `host:port`, naming the host.
pub(crate) async fn connect_socks5(
    stream: &mut TcpStream,
    host: &str,
    port: u16,
    proxy: &Proxy,
) -> Result<(), HttpError> {
    let methods: &[u8] = if proxy.credentials.is_some() {
        &[5, 2, 0, 2]
    } else {
        &[5, 1, 0]
    };
    stream.write_all(methods).await.map_err(failed)?;
    let mut choice = [0u8; 2];
    stream.read_exact(&mut choice).await.map_err(failed)?;
    if choice[0] != 5 {
        return Err(failed("that is not a SOCKS5 proxy"));
    }
    match choice[1] {
        0 => {}
        2 => {
            let (user, password) = proxy
                .credentials
                .as_ref()
                .ok_or_else(|| failed("the proxy wants a username and password"))?;
            let (user, password) = (user.as_bytes(), password.as_bytes());
            if user.len() > 255 || password.len() > 255 {
                return Err(failed("the proxy's username or password is too long"));
            }
            let mut login = vec![1, user.len() as u8];
            login.extend_from_slice(user);
            login.push(password.len() as u8);
            login.extend_from_slice(password);
            stream.write_all(&login).await.map_err(failed)?;
            let mut verdict = [0u8; 2];
            stream.read_exact(&mut verdict).await.map_err(failed)?;
            if verdict[1] != 0 {
                return Err(failed("the proxy refused the username or password"));
            }
        }
        _ => return Err(failed("the proxy accepts none of the ways to sign in")),
    }
    // An address goes as an address, a name as a name (which the proxy resolves).
    let literal = host.trim_start_matches('[').trim_end_matches(']');
    let mut request = vec![5, 1, 0];
    match literal.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => {
            request.push(1);
            request.extend_from_slice(&ip.octets());
        }
        Ok(std::net::IpAddr::V6(ip)) => {
            request.push(4);
            request.extend_from_slice(&ip.octets());
        }
        Err(_) => {
            if host.len() > 255 {
                return Err(failed("the host name is too long"));
            }
            request.push(3);
            request.push(host.len() as u8);
            request.extend_from_slice(host.as_bytes());
        }
    }
    request.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&request).await.map_err(failed)?;
    let mut reply = [0u8; 4];
    stream.read_exact(&mut reply).await.map_err(failed)?;
    if reply[1] != 0 {
        return Err(failed(format!(
            "the proxy could not connect (SOCKS5 code {})",
            reply[1]
        )));
    }
    // The address it bound: its length depends on the kind, and is not used.
    let skip = match reply[3] {
        1 => 4 + 2,
        4 => 16 + 2,
        3 => {
            let mut length = [0u8; 1];
            stream.read_exact(&mut length).await.map_err(failed)?;
            usize::from(length[0]) + 2
        }
        _ => return Err(failed("the proxy answered strangely")),
    };
    let mut rest = vec![0u8; skip];
    stream.read_exact(&mut rest).await.map_err(failed)?;
    Ok(())
}

/// A `multipart/form-data` body: `(content type, body)`, with some text
/// fields and one file.
pub fn multipart(fields: &[(&str, &str)], file: (&str, &str, &[u8])) -> (String, Vec<u8>) {
    let boundary = format!("----zui{}", hex::encode(rand::random::<[u8; 12]>()));
    let mut body = Vec::new();
    for (name, value) in fields {
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n")
                .as_bytes(),
        );
    }
    let (name, filename, bytes) = file;
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

#[cfg(test)]
pub mod testing {
    //! Servers on this machine to send to, and proxies to go through.

    use std::sync::{Arc, Mutex};

    use axum::body::Bytes;
    use axum::http::{HeaderMap, Method, Uri};
    use axum::Router;
    use tokio::net::TcpListener;

    /// What a mock server was asked.
    #[derive(Debug, Clone)]
    pub struct Seen {
        pub method: String,
        pub path: String,
        pub headers: Vec<(String, String)>,
        pub body: Vec<u8>,
    }

    impl Seen {
        pub fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.as_str())
        }

        pub fn json(&self) -> serde_json::Value {
            serde_json::from_slice(&self.body).unwrap_or(serde_json::Value::Null)
        }
    }

    /// A server that records every request and answers with `reply`.
    pub struct Mock {
        pub address: std::net::SocketAddr,
        pub seen: Arc<Mutex<Vec<Seen>>>,
    }

    impl Mock {
        pub fn url(&self) -> String {
            format!("http://{}", self.address)
        }

        pub fn requests(&self) -> Vec<Seen> {
            self.seen.lock().unwrap().clone()
        }
    }

    type Reply = dyn Fn(&Seen) -> (u16, String) + Send + Sync;

    pub async fn mock(reply: impl Fn(&Seen) -> (u16, String) + Send + Sync + 'static) -> Mock {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let reply: Arc<Reply> = Arc::new(reply);
        let recorder = Arc::clone(&seen);
        let app = Router::new().fallback(
            move |method: Method, uri: Uri, headers: HeaderMap, body: Bytes| {
                let recorder = Arc::clone(&recorder);
                let reply = Arc::clone(&reply);
                async move {
                    let request = Seen {
                        method: method.to_string(),
                        path: uri.to_string(),
                        headers: headers
                            .iter()
                            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                            .collect(),
                        body: body.to_vec(),
                    };
                    let (status, text) = reply(&request);
                    recorder.lock().unwrap().push(request);
                    (
                        axum::http::StatusCode::from_u16(status).unwrap(),
                        [("content-type", "application/json")],
                        text,
                    )
                }
            },
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Mock { address, seen }
    }

    /// A mock that always answers `200 {"ok":true,"result":{}}`.
    pub async fn ok() -> Mock {
        mock(|_| (200, r#"{"ok":true,"result":{}}"#.to_string())).await
    }

    /// A proxy that tunnels whatever it is asked, and remembers what for.
    pub struct Proxy {
        pub address: std::net::SocketAddr,
        pub asked: Arc<Mutex<Vec<String>>>,
    }

    /// An HTTP `CONNECT` proxy (wanting `user:pass` when `credentials`).
    pub async fn http_proxy(credentials: Option<&'static str>) -> Proxy {
        use base64::Engine;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let asked = Arc::new(Mutex::new(Vec::new()));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let log = Arc::clone(&asked);
        tokio::spawn(async move {
            while let Ok((mut client, _)) = listener.accept().await {
                let log = Arc::clone(&log);
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    let mut byte = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n") {
                        if client.read(&mut byte).await.unwrap_or(0) == 0 {
                            return;
                        }
                        head.push(byte[0]);
                    }
                    let text = String::from_utf8_lossy(&head).to_string();
                    let target = text.split_whitespace().nth(1).unwrap_or("").to_string();
                    if let Some(wanted) = credentials {
                        let token = base64::engine::general_purpose::STANDARD.encode(wanted);
                        if !text.contains(&format!("Proxy-Authorization: Basic {token}")) {
                            let _ = client
                                .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n")
                                .await;
                            return;
                        }
                    }
                    log.lock().unwrap().push(target.clone());
                    let Ok(mut upstream) = tokio::net::TcpStream::connect(&target).await else {
                        let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
                        return;
                    };
                    let _ = client
                        .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                        .await;
                    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
                });
            }
        });
        Proxy { address, asked }
    }

    /// A SOCKS5 proxy that is given the name, as `host:port`, and tunnels it
    /// to `redirect` (so a made-up name can stand for a mock).
    pub async fn socks_proxy(redirect: std::net::SocketAddr) -> Proxy {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let asked = Arc::new(Mutex::new(Vec::new()));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let log = Arc::clone(&asked);
        tokio::spawn(async move {
            while let Ok((mut client, _)) = listener.accept().await {
                let log = Arc::clone(&log);
                tokio::spawn(async move {
                    let mut greeting = [0u8; 2];
                    if client.read_exact(&mut greeting).await.is_err() {
                        return;
                    }
                    let mut methods = vec![0u8; usize::from(greeting[1])];
                    let _ = client.read_exact(&mut methods).await;
                    let _ = client.write_all(&[5, 0]).await;
                    let mut head = [0u8; 4];
                    if client.read_exact(&mut head).await.is_err() || head[3] != 3 {
                        return;
                    }
                    let mut length = [0u8; 1];
                    let _ = client.read_exact(&mut length).await;
                    let mut name = vec![0u8; usize::from(length[0])];
                    let _ = client.read_exact(&mut name).await;
                    let mut port = [0u8; 2];
                    let _ = client.read_exact(&mut port).await;
                    log.lock().unwrap().push(format!(
                        "{}:{}",
                        String::from_utf8_lossy(&name),
                        u16::from_be_bytes(port)
                    ));
                    let Ok(mut upstream) = tokio::net::TcpStream::connect(redirect).await else {
                        let _ = client.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]).await;
                        return;
                    };
                    let _ = client.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await;
                    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
                });
            }
        });
        Proxy { address, asked }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{http_proxy, mock, ok, socks_proxy};
    use super::*;

    const WAIT: Duration = Duration::from_secs(5);

    #[test]
    fn a_proxy_is_read_from_its_address() {
        assert_eq!(Proxy::parse("  ").unwrap(), None);
        let http = Proxy::parse("http://10.0.0.1:3128").unwrap().unwrap();
        assert_eq!(
            (http.kind, http.address.as_str()),
            (ProxyKind::Http, "10.0.0.1:3128")
        );
        assert_eq!(http.credentials, None);
        let socks = Proxy::parse("socks5://al%40i:p%3Ass@localhost:10808")
            .unwrap()
            .unwrap();
        assert_eq!(socks.kind, ProxyKind::Socks5);
        assert_eq!(socks.credentials, Some(("al@i".into(), "p:ss".into())));
        assert_eq!(
            Proxy::parse("socks5h://proxy").unwrap().unwrap().address,
            "proxy:1080"
        );
        assert_eq!(
            Proxy::parse("http://proxy").unwrap().unwrap().address,
            "proxy:8080"
        );
        assert_eq!(
            Proxy::parse("socks5://[::1]:1080")
                .unwrap()
                .unwrap()
                .address,
            "[::1]:1080"
        );
        for bad in ["ftp://proxy:21", "proxy:8080", "http://", "socks5://:1080"] {
            assert!(Proxy::parse(bad).is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn a_post_carries_its_body_and_headers() {
        let server = mock(|seen| (200, format!("{{\"echo\":{}}}", seen.body.len()))).await;
        let reply = post(
            &format!("{}/bot1/sendMessage", server.url()),
            "application/json",
            &[("X-Test", "yes")],
            br#"{"a":1}"#,
            None,
            WAIT,
        )
        .await
        .unwrap();
        assert_eq!(reply, br#"{"echo":7}"#);
        let seen = &server.requests()[0];
        assert_eq!(
            (seen.method.as_str(), seen.path.as_str()),
            ("POST", "/bot1/sendMessage")
        );
        assert_eq!(seen.header("x-test"), Some("yes"));
        assert_eq!(seen.header("content-type"), Some("application/json"));
        assert_eq!(seen.json()["a"], 1);
    }

    #[tokio::test]
    async fn a_refusal_is_its_status_and_a_dead_address_is_a_failure() {
        let server = mock(|_| (401, r#"{"ok":false}"#.to_string())).await;
        let refused = post(&server.url(), "application/json", &[], b"{}", None, WAIT).await;
        assert_eq!(refused, Err(HttpError::Status(401)));
        let nothing = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dead = format!("http://{}", nothing.local_addr().unwrap());
        drop(nothing);
        let failure = post(&dead, "application/json", &[], b"{}", None, WAIT).await;
        assert!(matches!(failure, Err(HttpError::Failed(_))), "{failure:?}");
    }

    #[tokio::test]
    async fn an_http_proxy_is_asked_for_a_tunnel_and_for_its_password() {
        let server = ok().await;
        let proxy = http_proxy(Some("bot:secret")).await;
        let want = Proxy::parse(&format!("http://bot:secret@{}", proxy.address)).unwrap();
        let reply = post(
            &format!("{}/x", server.url()),
            "application/json",
            &[],
            b"{}",
            want.as_ref(),
            WAIT,
        )
        .await
        .unwrap();
        assert_eq!(reply, br#"{"ok":true,"result":{}}"#);
        assert_eq!(
            proxy.asked.lock().unwrap().as_slice(),
            [server.address.to_string()]
        );
        assert_eq!(server.requests().len(), 1);

        // Without the password the proxy says no, and says so.
        let anonymous = Proxy::parse(&format!("http://{}", proxy.address)).unwrap();
        let refused = post(
            &server.url(),
            "application/json",
            &[],
            b"{}",
            anonymous.as_ref(),
            WAIT,
        )
        .await;
        match refused {
            Err(HttpError::Failed(message)) => assert!(message.contains("username"), "{message}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(server.requests().len(), 1, "nothing got through");
    }

    #[tokio::test]
    async fn a_socks_proxy_is_given_the_name_to_reach() {
        let server = ok().await;
        let proxy = socks_proxy(server.address).await;
        // The name means nothing here: only the proxy knows where it goes.
        let want = Proxy::parse(&format!("socks5://{}", proxy.address)).unwrap();
        let reply = post(
            "http://api.telegram.test:8081/bot/getMe",
            "application/json",
            &[],
            b"{}",
            want.as_ref(),
            WAIT,
        )
        .await
        .unwrap();
        assert_eq!(reply, br#"{"ok":true,"result":{}}"#);
        assert_eq!(
            proxy.asked.lock().unwrap().as_slice(),
            ["api.telegram.test:8081"]
        );
        assert_eq!(
            server.requests()[0].header("host"),
            Some("api.telegram.test:8081")
        );
    }

    #[tokio::test]
    async fn a_dead_proxy_is_a_failure_not_a_hang() {
        let nothing = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dead = nothing.local_addr().unwrap();
        drop(nothing);
        let want = Proxy::parse(&format!("socks5://{dead}")).unwrap();
        let failure = post(
            "http://example.test/",
            "application/json",
            &[],
            b"{}",
            want.as_ref(),
            WAIT,
        )
        .await;
        assert!(matches!(failure, Err(HttpError::Failed(_))), "{failure:?}");
    }

    #[test]
    fn a_file_goes_up_in_a_multipart_body() {
        let (content_type, body) = multipart(
            &[("chat_id", "42"), ("caption", "backup")],
            ("document", "a.bin", b"\x00\x01data"),
        );
        let boundary = content_type
            .strip_prefix("multipart/form-data; boundary=")
            .unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains(&format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"chat_id\"\r\n\r\n42\r\n"
        )));
        assert!(text.contains("name=\"document\"; filename=\"a.bin\""));
        assert!(body.windows(6).any(|w| w == b"\x00\x01data"));
        assert!(text.ends_with(&format!("\r\n--{boundary}--\r\n")));
    }
}
