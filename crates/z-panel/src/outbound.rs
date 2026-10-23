//! Outgoing HTTP for what the panel asks of other servers besides messaging:
//! a certificate authority, Cloudflare's API, the release server. One
//! request per connection, over a proxy if one is given, with the status,
//! headers and body of any answer (an ACME server explains a refusal in the
//! body of a 4xx, and a nonce travels in a header).
//!
//! Names go to a proxy unresolved, as in [`crate::notify::http`], whose
//! handshakes are used here. Certificates are checked against the public
//! roots built into the binary, not the machine's store, which a small
//! server image may not have.

use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::notify::http::{connect_http, connect_socks5, HttpError, Proxy, ProxyKind};

/// How much of an answer is read unless a caller asks for more.
pub const DEFAULT_MAX: usize = 4 * 1024 * 1024;
/// The longest a header block may be.
const MAX_HEAD: usize = 64 * 1024;

/// An answer, whatever its status.
#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    /// Names in lower case.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn header_any(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    #[cfg(test)]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.header_any(name)
    }

    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    #[cfg(test)]
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// What to send and how patiently.
#[derive(Clone, Copy)]
pub struct Request<'a> {
    pub method: &'a str,
    pub url: &'a str,
    pub headers: &'a [(&'a str, &'a str)],
    /// A content type and the body.
    pub body: Option<(&'a str, &'a [u8])>,
    pub proxy: Option<&'a Proxy>,
    pub timeout: Duration,
    pub max_bytes: usize,
    /// How many redirects to follow (a download from a release server is
    /// sent on to a storage host); none by default.
    pub redirects: u8,
}

impl<'a> Request<'a> {
    #[cfg(test)]
    pub fn get(url: &'a str) -> Self {
        Self {
            method: "GET",
            url,
            headers: &[],
            body: None,
            proxy: None,
            timeout: Duration::from_secs(30),
            max_bytes: DEFAULT_MAX,
            redirects: 0,
        }
    }
}

fn failed(what: impl std::fmt::Display) -> HttpError {
    HttpError::Failed(what.to_string())
}

/// Send `request` and read the whole answer. The work lives on the heap: it
/// is a large future, and callers are many.
pub async fn send(request: Request<'_>) -> Result<Reply, HttpError> {
    Box::pin(send_now(request)).await
}

async fn send_now(request: Request<'_>) -> Result<Reply, HttpError> {
    let timeout = request.timeout;
    tokio::time::timeout(timeout, follow(request))
        .await
        .map_err(|_| failed(format!("timed out after {timeout:?}")))?
}

/// The answer to `request`, after any redirects it is allowed to follow. The
/// time limit covers all of them together.
async fn follow(request: Request<'_>) -> Result<Reply, HttpError> {
    let mut url = request.url.to_string();
    let mut method = request.method;
    let mut body = request.body;
    for hop in 0..=request.redirects {
        let reply = exchange(Request {
            url: &url,
            method,
            body,
            ..request
        })
        .await?;
        let moved = matches!(reply.status, 301 | 302 | 303 | 307 | 308);
        let Some(location) = reply.header_any("location").filter(|_| moved) else {
            return Ok(reply);
        };
        // Following none is the default: the redirect is then the answer.
        if request.redirects == 0 {
            return Ok(reply);
        }
        if hop == request.redirects {
            return Err(failed(format!("more than {} redirects", request.redirects)));
        }
        let next = url::Url::parse(&url)
            .and_then(|base| base.join(location))
            .map_err(|e| failed(format!("a redirect to {location:?}: {e}")))?;
        // A page that moved is fetched again with GET; 307 and 308 keep the request.
        if matches!(reply.status, 301..=303) {
            method = "GET";
            body = None;
        }
        url = next.to_string();
    }
    Err(failed("too many redirects"))
}

async fn exchange(request: Request<'_>) -> Result<Reply, HttpError> {
    let target = url::Url::parse(request.url).map_err(|e| failed(format!("bad URL: {e}")))?;
    let tls = match target.scheme() {
        "https" => true,
        "http" => false,
        other => return Err(failed(format!("unsupported URL scheme {other}"))),
    };
    let host = target
        .host_str()
        .ok_or_else(|| failed("the URL has no host"))?
        .to_string();
    let bare = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    let port = target
        .port_or_known_default()
        .ok_or_else(|| failed("the URL has no port"))?;

    let stream = match request.proxy {
        None => TcpStream::connect((bare.as_str(), port))
            .await
            .map_err(|e| failed(format!("connecting to {host}:{port}: {e}")))?,
        Some(proxy) => {
            let mut stream = TcpStream::connect(&proxy.address)
                .await
                .map_err(|e| failed(format!("connecting to the proxy {}: {e}", proxy.address)))?;
            match proxy.kind {
                ProxyKind::Http => connect_http(&mut stream, &host, port, proxy).await?,
                ProxyKind::Socks5 => connect_socks5(&mut stream, &host, port, proxy).await?,
            }
            stream
        }
    };
    let _ = stream.set_nodelay(true);
    if tls {
        let connector = tokio_rustls::TlsConnector::from(zero_net::fetch::tls_config());
        let name = rustls_pki_types::ServerName::try_from(bare.clone())
            .map_err(|e| failed(format!("bad host name: {e}")))?;
        let stream = connector
            .connect(name, stream)
            .await
            .map_err(|e| failed(format!("TLS to {host}: {e}")))?;
        converse(stream, &request, &target).await
    } else {
        converse(stream, &request, &target).await
    }
}

async fn converse<S>(
    mut stream: S,
    request: &Request<'_>,
    target: &url::Url,
) -> Result<Reply, HttpError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut path = target.path().to_string();
    if path.is_empty() {
        path.push('/');
    }
    if let Some(query) = target.query() {
        path.push('?');
        path.push_str(query);
    }
    let host = target.host_str().unwrap_or_default();
    let authority = match target.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    };
    let mut head = format!(
        "{} {path} HTTP/1.1\r\nHost: {authority}\r\nUser-Agent: z-ui/{}\r\nAccept: */*\r\nConnection: close\r\n",
        request.method,
        z_common::VERSION
    );
    for (name, value) in request.headers {
        if name.contains(['\r', '\n']) || value.contains(['\r', '\n']) {
            return Err(failed("a header has a line break in it"));
        }
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    match request.body {
        Some((content_type, body)) => head.push_str(&format!(
            "Content-Type: {content_type}\r\nContent-Length: {}\r\n",
            body.len()
        )),
        None if matches!(request.method, "POST" | "PUT" | "PATCH") => {
            head.push_str("Content-Length: 0\r\n")
        }
        None => {}
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await.map_err(failed)?;
    if let Some((_, body)) = request.body {
        stream.write_all(body).await.map_err(failed)?;
    }
    stream.flush().await.map_err(failed)?;
    read_reply(stream, request.method == "HEAD", request.max_bytes).await
}

/// A buffer over a stream that can be read by lines and by counts.
struct Source<S> {
    stream: S,
    buffer: Vec<u8>,
    ended: bool,
}

impl<S: AsyncRead + Unpin> Source<S> {
    async fn fill(&mut self) -> Result<bool, HttpError> {
        if self.ended {
            return Ok(false);
        }
        let mut chunk = [0u8; 8192];
        let n = self.stream.read(&mut chunk).await.map_err(failed)?;
        if n == 0 {
            self.ended = true;
            return Ok(false);
        }
        self.buffer.extend_from_slice(&chunk[..n]);
        Ok(true)
    }

    /// Up to and not including the next CRLF, which is consumed.
    async fn line(&mut self, limit: usize) -> Result<String, HttpError> {
        loop {
            if let Some(at) = self.buffer.windows(2).position(|w| w == b"\r\n") {
                let line: Vec<u8> = self.buffer.drain(..at + 2).take(at).collect();
                return Ok(String::from_utf8_lossy(&line).into_owned());
            }
            if self.buffer.len() > limit {
                return Err(failed("a line in the answer is too long"));
            }
            if !self.fill().await? {
                return Err(failed("the connection closed in the middle of the answer"));
            }
        }
    }

    /// Exactly `n` bytes.
    async fn take(&mut self, n: usize) -> Result<Vec<u8>, HttpError> {
        while self.buffer.len() < n {
            if !self.fill().await? {
                return Err(failed("the connection closed before the whole body came"));
            }
        }
        Ok(self.buffer.drain(..n).collect())
    }

    /// Everything until the server closes, at most `max` bytes.
    async fn rest(&mut self, max: usize) -> Result<Vec<u8>, HttpError> {
        while self.fill().await? {
            if self.buffer.len() > max {
                return Err(failed(format!("the answer is longer than {max} bytes")));
            }
        }
        Ok(std::mem::take(&mut self.buffer))
    }
}

async fn read_reply<S>(stream: S, head_only: bool, max_bytes: usize) -> Result<Reply, HttpError>
where
    S: AsyncRead + Unpin,
{
    let mut source = Source {
        stream,
        buffer: Vec::new(),
        ended: false,
    };
    let status_line = source.line(MAX_HEAD).await?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .filter(|_| status_line.starts_with("HTTP/"))
        .ok_or_else(|| failed("that is not an HTTP answer"))?;
    let mut headers = Vec::new();
    let mut head_size = status_line.len();
    loop {
        let line = source.line(MAX_HEAD).await?;
        if line.is_empty() {
            break;
        }
        head_size += line.len();
        if head_size > MAX_HEAD {
            return Err(failed("the answer's headers are too long"));
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    let find = |name: &str| {
        headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };
    let no_body = head_only || status / 100 == 1 || matches!(status, 204 | 304);
    let body = if no_body {
        Vec::new()
    } else if find("transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked"))
    {
        let mut body = Vec::new();
        loop {
            let size_line = source.line(1024).await?;
            let size_text = size_line.split(';').next().unwrap_or("").trim();
            let size = usize::from_str_radix(size_text, 16)
                .map_err(|_| failed("a chunk of the answer has no size"))?;
            if size == 0 {
                // Trailers, if any, up to the blank line.
                while !source.line(MAX_HEAD).await?.is_empty() {}
                break;
            }
            if body.len() + size > max_bytes {
                return Err(failed(format!(
                    "the answer is longer than {max_bytes} bytes"
                )));
            }
            body.extend(source.take(size).await?);
            if !source.take(2).await?.eq(b"\r\n") {
                return Err(failed("a chunk of the answer is not ended properly"));
            }
        }
        body
    } else if let Some(length) = find("content-length").and_then(|v| v.parse::<usize>().ok()) {
        if length > max_bytes {
            return Err(failed(format!(
                "the answer is longer than {max_bytes} bytes"
            )));
        }
        source.take(length).await?
    } else {
        source.rest(max_bytes).await?
    };
    Ok(Reply {
        status,
        headers,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    /// A server that answers one request with `raw`, and gives back what it
    /// was sent.
    async fn serve_once(raw: &'static [u8]) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/path?q=1", listener.local_addr().unwrap());
        let seen = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut got = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = stream.read(&mut chunk).await.unwrap();
                got.extend_from_slice(&chunk[..n]);
                // The head ends at a blank line; a body follows by its length.
                if let Some(end) = got.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&got[..end]).to_ascii_lowercase();
                    let length = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length: "))
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(0);
                    if got.len() >= end + 4 + length {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            stream.write_all(raw).await.unwrap();
            stream.shutdown().await.unwrap();
            String::from_utf8_lossy(&got).into_owned()
        });
        (url, seen)
    }

    #[tokio::test]
    async fn an_answer_is_read_with_its_status_headers_and_body_whatever_the_status() {
        let (url, seen) = serve_once(
            b"HTTP/1.1 400 Bad Request\r\nReplay-Nonce: abc\r\nContent-Type: application/problem+json\r\nContent-Length: 18\r\n\r\n{\"type\":\"badNonc\"}",
        )
        .await;
        let reply = send(Request {
            method: "POST",
            headers: &[("X-Test", "1")],
            body: Some(("application/jose+json", b"{\"a\":1}")),
            ..Request::get(&url)
        })
        .await
        .unwrap();
        assert_eq!(reply.status, 400);
        assert!(!reply.is_success());
        assert_eq!(reply.header("replay-nonce"), Some("abc"));
        assert_eq!(
            reply.header("Content-Type"),
            Some("application/problem+json")
        );
        assert_eq!(reply.text(), "{\"type\":\"badNonc\"}");
        let request = seen.await.unwrap();
        assert!(
            request.starts_with("POST /path?q=1 HTTP/1.1\r\n"),
            "{request}"
        );
        assert!(request.contains("X-Test: 1\r\n") && request.contains("Content-Length: 7\r\n"));
        assert!(request.contains("Content-Type: application/jose+json\r\n"));
        assert!(request.ends_with("{\"a\":1}"));
    }

    #[tokio::test]
    async fn chunked_bodies_and_bodies_that_end_at_the_close_are_read() {
        let (url, _) = serve_once(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6;ext=1\r\n world\r\n0\r\nX-Trailer: 1\r\n\r\n",
        )
        .await;
        let reply = send(Request::get(&url)).await.unwrap();
        assert_eq!(reply.text(), "hello world");

        let (url, _) = serve_once(b"HTTP/1.0 200 OK\r\n\r\nuntil the end").await;
        assert_eq!(
            send(Request::get(&url)).await.unwrap().text(),
            "until the end"
        );
    }

    #[tokio::test]
    async fn a_head_request_has_no_body_and_a_big_answer_is_refused() {
        let (url, _) =
            serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nLocation: /x\r\n\r\n").await;
        let reply = send(Request {
            method: "HEAD",
            ..Request::get(&url)
        })
        .await
        .unwrap();
        assert!(reply.body.is_empty() && reply.header("location") == Some("/x"));

        let (url, _) = serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n").await;
        let error = send(Request {
            max_bytes: 10,
            ..Request::get(&url)
        })
        .await
        .unwrap_err();
        assert!(error.to_string().contains("longer than 10"), "{error}");
    }

    #[tokio::test]
    async fn nonsense_and_a_cut_off_answer_are_errors_and_a_header_cannot_smuggle_a_line() {
        let (url, _) = serve_once(b"SSH-2.0-OpenSSH\r\n\r\n").await;
        assert!(send(Request::get(&url)).await.is_err());
        let (url, _) = serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: 50\r\n\r\nshort").await;
        assert!(send(Request::get(&url)).await.is_err());
        let (url, _) = serve_once(b"HTTP/1.1 200 OK\r\n\r\n").await;
        let error = send(Request {
            headers: &[("X", "a\r\nInjected: 1")],
            ..Request::get(&url)
        })
        .await
        .unwrap_err();
        assert!(error.to_string().contains("line break"), "{error}");
    }

    /// A server that answers each of several connections with one of `raws`.
    async fn serve_each(raws: Vec<&'static [u8]>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            for raw in raws {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut buffer = [0u8; 4096];
                let _ = stream.read(&mut buffer).await;
                // The client may already have what it came for and hung up.
                let _ = stream.write_all(raw).await;
                let _ = stream.shutdown().await;
            }
        });
        base
    }

    #[tokio::test]
    async fn redirects_are_followed_only_as_far_as_allowed_and_a_moved_page_is_fetched_with_get() {
        let base = serve_each(vec![
            b"HTTP/1.1 302 Found\r\nLocation: /second\r\nContent-Length: 0\r\n\r\n",
            b"HTTP/1.1 307 Temporary Redirect\r\nLocation: /third\r\nContent-Length: 0\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nhere",
        ])
        .await;
        let url = format!("{base}/first");
        let reply = send(Request {
            redirects: 3,
            ..Request::get(&url)
        })
        .await
        .unwrap();
        assert_eq!(reply.text(), "here");

        // By default nothing is followed: the redirect is the answer.
        let base = serve_each(vec![
            b"HTTP/1.1 302 Found\r\nLocation: /x\r\nContent-Length: 0\r\n\r\n",
        ])
        .await;
        let reply = send(Request::get(&format!("{base}/a"))).await.unwrap();
        assert_eq!((reply.status, reply.header("location")), (302, Some("/x")));

        // And a loop ends.
        let base = serve_each(vec![
            b"HTTP/1.1 302 Found\r\nLocation: /a\r\nContent-Length: 0\r\n\r\n",
            b"HTTP/1.1 302 Found\r\nLocation: /a\r\nContent-Length: 0\r\n\r\n",
            b"HTTP/1.1 302 Found\r\nLocation: /a\r\nContent-Length: 0\r\n\r\n",
        ])
        .await;
        let error = send(Request {
            redirects: 2,
            ..Request::get(&format!("{base}/a"))
        })
        .await
        .unwrap_err();
        assert!(error.to_string().contains("redirects"), "{error}");
    }

    #[tokio::test]
    async fn a_request_goes_through_a_proxy_that_is_asked_for_the_name() {
        use crate::notify::http::testing as net;
        let (origin, _) = serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi").await;
        let proxy = net::http_proxy(None).await;
        let parsed = Proxy::parse(&format!("http://{}", proxy.address))
            .unwrap()
            .unwrap();
        let reply = send(Request {
            proxy: Some(&parsed),
            ..Request::get(&origin)
        })
        .await
        .unwrap();
        assert_eq!(reply.text(), "hi");
        let asked = proxy.asked.lock().unwrap().clone();
        assert_eq!(asked.len(), 1, "{asked:?}");
        assert!(origin.contains(&asked[0]), "{origin} {asked:?}");
    }
}
