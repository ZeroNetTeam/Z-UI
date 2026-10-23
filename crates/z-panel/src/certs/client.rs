//! The ACME client's HTTP, made over the panel's own outgoing layer
//! ([`crate::outbound`]): so a proxy applies, the certificate authority's
//! certificate is checked against the roots built into the binary, and a
//! test can stand a plain-HTTP authority on this machine.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use bytes::Bytes;
use http::header::{HeaderName, HeaderValue, CONNECTION, CONTENT_LENGTH, CONTENT_TYPE, HOST};
use http_body_util::{BodyExt, Full};
use instant_acme::{BodyWrapper, BytesResponse, HttpClient};

use crate::notify::http::Proxy;
use crate::outbound::{self, Request};

/// An ACME answer can carry a chain of certificates; never more than this.
const MAX_REPLY: usize = 2 * 1024 * 1024;

pub struct Client {
    pub proxy: Option<Proxy>,
}

fn other(message: impl std::fmt::Display) -> instant_acme::Error {
    instant_acme::Error::Other(Box::new(std::io::Error::other(message.to_string())))
}

impl HttpClient for Client {
    fn request(
        &self,
        req: http::Request<BodyWrapper<Bytes>>,
    ) -> Pin<Box<dyn Future<Output = Result<BytesResponse, instant_acme::Error>> + Send>> {
        let proxy = self.proxy.clone();
        Box::pin(async move {
            let (parts, body) = req.into_parts();
            let body = body.collect().await.map_err(other)?.to_bytes();
            let url = parts.uri.to_string();
            let content_type = parts
                .headers
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/jose+json")
                .to_string();
            // What the request layer writes itself is not passed twice.
            let headers: Vec<(String, String)> = parts
                .headers
                .iter()
                .filter(|(name, _)| {
                    ![HOST, CONTENT_LENGTH, CONTENT_TYPE, CONNECTION].contains(name)
                })
                .filter_map(|(name, value)| {
                    Some((name.as_str().to_string(), value.to_str().ok()?.to_string()))
                })
                .collect();
            let pairs: Vec<(&str, &str)> = headers
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str()))
                .collect();
            let method = parts.method.as_str();
            let has_body = !(body.is_empty() && matches!(method, "GET" | "HEAD"));
            let reply = outbound::send(Request {
                method,
                url: &url,
                headers: &pairs,
                body: has_body.then_some((content_type.as_str(), &body[..])),
                proxy: proxy.as_ref(),
                timeout: Duration::from_secs(30),
                max_bytes: MAX_REPLY,
                redirects: 0,
            })
            .await
            .map_err(other)?;

            let mut response = http::Response::builder().status(reply.status);
            for (name, value) in &reply.headers {
                if let (Ok(name), Ok(value)) = (
                    HeaderName::from_bytes(name.as_bytes()),
                    HeaderValue::from_str(value),
                ) {
                    response = response.header(name, value);
                }
            }
            let response = response
                .body(Full::new(Bytes::from(reply.body)))
                .map_err(other)?;
            Ok(BytesResponse::from(response))
        })
    }
}
