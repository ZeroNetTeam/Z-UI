//! A certificate authority and a Cloudflare that exist only in the test's
//! process, speaking enough of ACME (RFC 8555, 8737, 8738) and of the DNS
//! API for the panel's client to do everything it does for real: make an
//! account, order, be asked for a challenge and have it checked, finalize,
//! download. Signatures on requests are not verified; everything the
//! authority is told is.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Router;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rcgen::{CertificateParams, CertificateSigningRequestParams, IsCa, Issuer, KeyPair};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;

use super::solve::alpn_server_name;

fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

fn unb64(text: &str) -> Vec<u8> {
    URL_SAFE_NO_PAD.decode(text).unwrap_or_default()
}

// ---------------------------------------------------------------- Cloudflare

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Txt {
    pub id: String,
    pub zone: String,
    pub name: String,
    pub content: String,
}

pub struct Cloudflare {
    pub base: String,
    pub records: Arc<Mutex<Vec<Txt>>>,
    pub calls: Mutex<Vec<String>>,
    token: String,
    /// `(id, name)` of the zones this account has.
    zones: Vec<(String, String)>,
}

impl Cloudflare {
    pub async fn start(token: &str, zones: &[&str]) -> Arc<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let me = Arc::new(Self {
            base: format!("http://{}", listener.local_addr().unwrap()),
            records: Arc::default(),
            calls: Mutex::default(),
            token: token.into(),
            zones: zones
                .iter()
                .enumerate()
                .map(|(i, name)| (format!("zone{i}"), (*name).to_string()))
                .collect(),
        });
        let app = Router::new()
            .fallback(Self::handle)
            .with_state(Arc::clone(&me));
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        me
    }

    async fn handle(
        State(me): State<Arc<Self>>,
        method: Method,
        uri: Uri,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        me.calls
            .lock()
            .unwrap()
            .push(format!("{method} {}", uri.path()));
        let fail = |status: StatusCode, message: &str| {
            (
                status,
                axum::Json(json!({"success": false, "errors": [{"message": message}]})),
            )
                .into_response()
        };
        let ok =
            |result: Value| axum::Json(json!({"success": true, "result": result})).into_response();
        let authorised = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v == format!("Bearer {}", me.token));
        if !authorised {
            return fail(StatusCode::FORBIDDEN, "Authentication error");
        }
        let path = uri.path().trim_start_matches('/').to_string();
        let parts: Vec<&str> = path.split('/').collect();
        match (method.as_str(), parts.as_slice()) {
            ("GET", ["zones"]) => {
                let wanted = uri
                    .query()
                    .and_then(|q| {
                        q.split('&').find_map(|p| p.strip_prefix("name=")).map(|v| {
                            percent_encoding::percent_decode_str(v)
                                .decode_utf8_lossy()
                                .into_owned()
                        })
                    })
                    .unwrap_or_default();
                ok(json!(me
                    .zones
                    .iter()
                    .filter(|(_, name)| *name == wanted)
                    .map(|(id, name)| json!({"id": id, "name": name}))
                    .collect::<Vec<_>>()))
            }
            ("POST", ["zones", zone, "dns_records"]) => {
                let body: Value = serde_json::from_slice(&body).unwrap_or_default();
                let mut records = me.records.lock().unwrap();
                let id = format!("rec{}", records.len() + 1);
                records.push(Txt {
                    id: id.clone(),
                    zone: (*zone).to_string(),
                    name: body["name"].as_str().unwrap_or_default().to_string(),
                    content: body["content"].as_str().unwrap_or_default().to_string(),
                });
                ok(json!({"id": id}))
            }
            ("DELETE", ["zones", _, "dns_records", id]) => {
                me.records.lock().unwrap().retain(|r| r.id != *id);
                ok(json!({"id": id}))
            }
            _ => fail(StatusCode::NOT_FOUND, "no such endpoint"),
        }
    }
}

// ----------------------------------------------------------------- authority

#[derive(Default)]
struct Authz {
    identifier: String,
    is_ip: bool,
    wildcard: bool,
    token: String,
    status: String,
    error: Option<String>,
    thumb: String,
}

#[derive(Default)]
struct Order {
    authz: Vec<usize>,
    identifiers: Vec<(String, String)>,
    finalized: bool,
    certificate: Option<String>,
}

#[derive(Default)]
struct Inner {
    /// Account url → the thumbprint of its key.
    accounts: HashMap<String, String>,
    authz: Vec<Authz>,
    orders: Vec<Order>,
    nonce: u64,
    log: Vec<String>,
    profiles: Vec<Option<String>>,
    eab_kids: Vec<String>,
    contacts: Vec<Vec<String>>,
    http_port: u16,
    tls_port: u16,
    refuse_with: Option<String>,
}

pub struct Ca {
    pub base: String,
    require_eab: bool,
    dns: Arc<Mutex<Vec<Txt>>>,
    issuer: (CertificateParams, KeyPair),
    ca_pem: String,
    inner: Mutex<Inner>,
}

impl Ca {
    pub async fn start(require_eab: bool, dns: Arc<Mutex<Vec<Txt>>>) -> Arc<Self> {
        let key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Mock CA");
        params.key_usages = vec![
            rcgen::KeyUsagePurpose::KeyCertSign,
            rcgen::KeyUsagePurpose::CrlSign,
        ];
        let ca_pem = params.self_signed(&key).unwrap().pem();
        // The base address is needed inside the handlers, so it is set once
        // the listener exists, through a cell the handlers read.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let me = Arc::new(Self {
            base,
            require_eab,
            dns,
            issuer: (params, key),
            ca_pem,
            inner: Mutex::default(),
        });
        let app = Router::new()
            .fallback(Self::handle)
            .with_state(Arc::clone(&me));
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        me
    }

    /// Where the challenges will be checked on this machine.
    pub fn listen_ports(&self, http: u16, tls: u16) {
        let mut inner = self.inner.lock().unwrap();
        inner.http_port = http;
        inner.tls_port = tls;
    }

    /// From now on every validation fails with this said.
    pub fn refuse(&self, detail: Option<&str>) {
        self.inner.lock().unwrap().refuse_with = detail.map(str::to_string);
    }

    pub fn log(&self) -> Vec<String> {
        self.inner.lock().unwrap().log.clone()
    }

    pub fn count(&self, what: &str) -> usize {
        self.log().iter().filter(|l| l.contains(what)).count()
    }

    pub fn profiles(&self) -> Vec<Option<String>> {
        self.inner.lock().unwrap().profiles.clone()
    }

    pub fn eab_kids(&self) -> Vec<String> {
        self.inner.lock().unwrap().eab_kids.clone()
    }

    pub fn contacts(&self) -> Vec<Vec<String>> {
        self.inner.lock().unwrap().contacts.clone()
    }

    /// The identifiers of every order, as sent.
    pub fn ordered(&self) -> Vec<Vec<String>> {
        self.inner
            .lock()
            .unwrap()
            .orders
            .iter()
            .map(|o| o.identifiers.iter().map(|(_, v)| v.clone()).collect())
            .collect()
    }

    fn reply(&self, status: StatusCode, body: Value, location: Option<String>) -> Response {
        let nonce = {
            let mut inner = self.inner.lock().unwrap();
            inner.nonce += 1;
            format!("nonce-{}", inner.nonce)
        };
        let mut response = (status, axum::Json(body)).into_response();
        let headers = response.headers_mut();
        headers.insert("replay-nonce", HeaderValue::from_str(&nonce).unwrap());
        if let Some(location) = location {
            headers.insert("location", HeaderValue::from_str(&location).unwrap());
        }
        response
    }

    fn problem(&self, status: StatusCode, kind: &str, detail: &str) -> Response {
        let mut response = self.reply(
            status,
            json!({"type": format!("urn:ietf:params:acme:error:{kind}"), "detail": detail}),
            None,
        );
        response.headers_mut().insert(
            "content-type",
            HeaderValue::from_static("application/problem+json"),
        );
        response
    }

    async fn handle(
        State(me): State<Arc<Self>>,
        method: Method,
        uri: Uri,
        body: Bytes,
    ) -> Response {
        let path = uri.path().to_string();
        me.inner
            .lock()
            .unwrap()
            .log
            .push(format!("{method} {path}"));
        let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        let base = me.base.clone();
        match (method.as_str(), parts.as_slice()) {
            ("GET", ["directory"]) => me.reply(
                StatusCode::OK,
                json!({
                    "newNonce": format!("{base}/new-nonce"),
                    "newAccount": format!("{base}/new-acct"),
                    "newOrder": format!("{base}/new-order"),
                    "revokeCert": format!("{base}/revoke"),
                    "keyChange": format!("{base}/key-change"),
                    "meta": {
                        "externalAccountRequired": me.require_eab,
                        "profiles": {"classic": "the usual", "shortlived": "six days"},
                    },
                }),
                None,
            ),
            ("HEAD" | "GET", ["new-nonce"]) => {
                let mut response = me.reply(StatusCode::OK, json!({}), None);
                *response.status_mut() = StatusCode::OK;
                response
            }
            ("POST", [route, rest @ ..]) => {
                let (header, payload) = jws(&body);
                match (*route, rest) {
                    ("new-acct", []) => me.new_account(&header, &payload),
                    ("new-order", []) => me.new_order(&header, &payload),
                    ("authz", [i]) => me.authz(i.parse().unwrap_or(usize::MAX)),
                    ("chall", [i, kind]) => {
                        me.challenge(&header, i.parse().unwrap_or(usize::MAX), kind)
                            .await
                    }
                    ("order", [i]) => me.order(i.parse().unwrap_or(usize::MAX)),
                    ("finalize", [i]) => me.finalize(i.parse().unwrap_or(usize::MAX), &payload),
                    ("cert", [i]) => me.certificate(i.parse().unwrap_or(usize::MAX)),
                    _ => me.problem(StatusCode::NOT_FOUND, "malformed", "no such route"),
                }
            }
            _ => me.problem(StatusCode::NOT_FOUND, "malformed", "no such route"),
        }
    }

    fn new_account(&self, header: &Value, payload: &Value) -> Response {
        let Some(jwk) = header.get("jwk") else {
            return self.problem(StatusCode::BAD_REQUEST, "malformed", "no jwk");
        };
        let eab = payload.get("externalAccountBinding");
        if self.require_eab && eab.is_none() {
            return self.problem(
                StatusCode::BAD_REQUEST,
                "externalAccountRequired",
                "this authority gives accounts only to those with a key",
            );
        }
        let thumbprint = b64(&Sha256::digest(
            format!(
                r#"{{"crv":"{}","kty":"{}","x":"{}","y":"{}"}}"#,
                jwk["crv"].as_str().unwrap_or_default(),
                jwk["kty"].as_str().unwrap_or_default(),
                jwk["x"].as_str().unwrap_or_default(),
                jwk["y"].as_str().unwrap_or_default(),
            )
            .as_bytes(),
        ));
        let mut inner = self.inner.lock().unwrap();
        let url = format!("{}/acct/{}", self.base, inner.accounts.len() + 1);
        inner.accounts.insert(url.clone(), thumbprint);
        if let Some(eab) = eab {
            let protected: Value =
                serde_json::from_slice(&unb64(eab["protected"].as_str().unwrap_or_default()))
                    .unwrap_or_default();
            inner
                .eab_kids
                .push(protected["kid"].as_str().unwrap_or_default().to_string());
        }
        inner.contacts.push(
            payload["contact"]
                .as_array()
                .map(|c| {
                    c.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
        );
        drop(inner);
        self.reply(
            StatusCode::CREATED,
            json!({"status": "valid", "contact": payload["contact"]}),
            Some(url),
        )
    }

    fn new_order(&self, header: &Value, payload: &Value) -> Response {
        let mut inner = self.inner.lock().unwrap();
        let account = header["kid"].as_str().unwrap_or_default();
        let Some(thumb) = inner.accounts.get(account).cloned() else {
            drop(inner);
            return self.problem(
                StatusCode::BAD_REQUEST,
                "accountDoesNotExist",
                "unknown account",
            );
        };
        inner
            .profiles
            .push(payload["profile"].as_str().map(str::to_string));
        let mut order = Order::default();
        for identifier in payload["identifiers"]
            .as_array()
            .cloned()
            .unwrap_or_default()
        {
            let kind = identifier["type"].as_str().unwrap_or_default().to_string();
            let value = identifier["value"].as_str().unwrap_or_default().to_string();
            order.identifiers.push((kind.clone(), value.clone()));
            let wildcard = value.starts_with("*.");
            let id = inner.authz.len();
            inner.authz.push(Authz {
                identifier: value.trim_start_matches("*.").to_string(),
                is_ip: kind == "ip",
                wildcard,
                token: b64(&rand::random::<[u8; 16]>()),
                status: "pending".into(),
                error: None,
                thumb: thumb.clone(),
            });
            order.authz.push(id);
        }
        inner.orders.push(order);
        let index = inner.orders.len() - 1;
        let order = &inner.orders[index];
        let body = json!({
            "status": "pending",
            "identifiers": order.identifiers.iter().map(|(t, v)| json!({"type": t, "value": v})).collect::<Vec<_>>(),
            "authorizations": order.authz.iter().map(|i| format!("{}/authz/{i}", self.base)).collect::<Vec<_>>(),
            "finalize": format!("{}/finalize/{index}", self.base),
        });
        drop(inner);
        self.reply(
            StatusCode::CREATED,
            body,
            Some(format!("{}/order/{index}", self.base)),
        )
    }

    fn authz(&self, index: usize) -> Response {
        let inner = self.inner.lock().unwrap();
        let Some(a) = inner.authz.get(index) else {
            drop(inner);
            return self.problem(StatusCode::NOT_FOUND, "malformed", "no such authorization");
        };
        let mut kinds = vec!["http-01", "tls-alpn-01"];
        if !a.is_ip {
            kinds.push("dns-01");
        }
        if a.wildcard {
            kinds = vec!["dns-01"];
        }
        let challenges: Vec<Value> = kinds
            .iter()
            .map(|kind| {
                let mut challenge = json!({
                    "type": kind,
                    "url": format!("{}/chall/{index}/{kind}", self.base),
                    "token": a.token,
                    "status": if a.status == "pending" { "pending" } else { a.status.as_str() },
                });
                if let Some(error) = &a.error {
                    challenge["error"] = json!({
                        "type": "urn:ietf:params:acme:error:unauthorized",
                        "detail": error,
                    });
                }
                challenge
            })
            .collect();
        let mut body = json!({
            "status": a.status,
            "identifier": {"type": if a.is_ip { "ip" } else { "dns" }, "value": a.identifier},
            "challenges": challenges,
        });
        if a.wildcard {
            body["wildcard"] = json!(true);
        }
        drop(inner);
        self.reply(StatusCode::OK, body, None)
    }

    async fn challenge(&self, header: &Value, index: usize, kind: &str) -> Response {
        let (authz, http_port, tls_port, refuse) = {
            let inner = self.inner.lock().unwrap();
            let Some(a) = inner.authz.get(index) else {
                drop(inner);
                return self.problem(StatusCode::NOT_FOUND, "malformed", "no such challenge");
            };
            let _ = header;
            (
                (
                    a.identifier.clone(),
                    a.is_ip,
                    a.token.clone(),
                    a.thumb.clone(),
                ),
                inner.http_port,
                inner.tls_port,
                inner.refuse_with.clone(),
            )
        };
        let (identifier, is_ip, token, thumb) = authz;
        let key_authorization = format!("{token}.{thumb}");
        let outcome: Result<(), String> = match refuse {
            Some(detail) => Err(detail),
            None => match kind {
                "http-01" => self.check_http(http_port, &token, &key_authorization).await,
                "tls-alpn-01" => {
                    self.check_alpn(tls_port, &identifier, is_ip, &key_authorization)
                        .await
                }
                _ => self.check_dns(&identifier, &key_authorization),
            },
        };
        let mut inner = self.inner.lock().unwrap();
        let a = &mut inner.authz[index];
        match outcome {
            Ok(()) => {
                a.status = "valid".into();
                drop(inner);
                self.reply(
                    StatusCode::OK,
                    json!({"type": kind, "url": format!("{}/chall/{index}/{kind}", self.base), "token": token, "status": "valid"}),
                    None,
                )
            }
            Err(detail) => {
                a.status = "invalid".into();
                a.error = Some(detail.clone());
                drop(inner);
                self.reply(
                    StatusCode::OK,
                    json!({"type": kind, "url": format!("{}/chall/{index}/{kind}", self.base), "token": token, "status": "invalid",
                        "error": {"type": "urn:ietf:params:acme:error:unauthorized", "detail": detail}}),
                    None,
                )
            }
        }
    }

    async fn check_http(&self, port: u16, token: &str, expected: &str) -> Result<(), String> {
        let url = format!("http://127.0.0.1:{port}/.well-known/acme-challenge/{token}");
        let reply = crate::outbound::send(crate::outbound::Request::get(&url))
            .await
            .map_err(|e| format!("Fetching {url}: {e}"))?;
        if reply.status == 200 && reply.text() == expected {
            Ok(())
        } else {
            Err(format!("Invalid response from {url}: {}", reply.status))
        }
    }

    async fn check_alpn(
        &self,
        port: u16,
        identifier: &str,
        is_ip: bool,
        expected: &str,
    ) -> Result<(), String> {
        let name = if is_ip {
            alpn_server_name(identifier)
        } else {
            identifier.to_string()
        };
        let config = crate::tls::insecure_client(&[b"acme-tls/1"]);
        let tcp = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .map_err(|e| format!("Connecting to port {port}: {e}"))?;
        let stream = tokio_rustls::TlsConnector::from(config)
            .connect(
                rustls_pki_types::ServerName::try_from(name.clone()).unwrap(),
                tcp,
            )
            .await
            .map_err(|e| format!("TLS handshake for {name}: {e}"))?;
        let leaf = stream
            .get_ref()
            .1
            .peer_certificates()
            .and_then(|c| c.first().cloned())
            .ok_or("no certificate")?;
        let (_, cert) =
            x509_parser::parse_x509_certificate(leaf.as_ref()).map_err(|e| e.to_string())?;
        let extension = cert
            .extensions()
            .iter()
            .find(|e| e.oid.to_id_string() == "1.3.6.1.5.5.7.1.31")
            .ok_or("the certificate has no acmeIdentifier extension")?;
        if !extension.critical {
            return Err("the acmeIdentifier extension must be critical".into());
        }
        let digest = Sha256::digest(expected.as_bytes());
        if extension.value.ends_with(&digest) {
            Ok(())
        } else {
            Err("the acmeIdentifier does not hold the key authorization's digest".into())
        }
    }

    fn check_dns(&self, identifier: &str, expected: &str) -> Result<(), String> {
        let value = b64(&Sha256::digest(expected.as_bytes()));
        let name = format!("_acme-challenge.{identifier}");
        let found = self
            .dns
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.name == name && r.content == value);
        if found {
            Ok(())
        } else {
            Err(format!("No TXT record {name} with the expected value"))
        }
    }

    fn order_body(&self, index: usize) -> Option<Value> {
        let inner = self.inner.lock().unwrap();
        let order = inner.orders.get(index)?;
        let statuses: Vec<&str> = order
            .authz
            .iter()
            .map(|i| inner.authz[*i].status.as_str())
            .collect();
        let status = if statuses.contains(&"invalid") {
            "invalid"
        } else if order.finalized {
            "valid"
        } else if statuses.iter().all(|s| *s == "valid") {
            "ready"
        } else {
            "pending"
        };
        let mut body = json!({
            "status": status,
            "identifiers": order.identifiers.iter().map(|(t, v)| json!({"type": t, "value": v})).collect::<Vec<_>>(),
            "authorizations": order.authz.iter().map(|i| format!("{}/authz/{i}", self.base)).collect::<Vec<_>>(),
            "finalize": format!("{}/finalize/{index}", self.base),
        });
        if status == "valid" {
            body["certificate"] = json!(format!("{}/cert/{index}", self.base));
        }
        Some(body)
    }

    fn order(&self, index: usize) -> Response {
        match self.order_body(index) {
            Some(body) => self.reply(StatusCode::OK, body, None),
            None => self.problem(StatusCode::NOT_FOUND, "malformed", "no such order"),
        }
    }

    fn finalize(&self, index: usize, payload: &Value) -> Response {
        let Some(body) = self.order_body(index) else {
            return self.problem(StatusCode::NOT_FOUND, "malformed", "no such order");
        };
        if body["status"] != "ready" {
            return self.problem(
                StatusCode::FORBIDDEN,
                "orderNotReady",
                "the order is not ready",
            );
        }
        let der = rustls_pki_types::CertificateSigningRequestDer::from(unb64(
            payload["csr"].as_str().unwrap_or_default(),
        ));
        let Ok(request) = CertificateSigningRequestParams::from_der(&der) else {
            return self.problem(
                StatusCode::BAD_REQUEST,
                "badCSR",
                "the request cannot be read",
            );
        };
        let mut asked: Vec<String> = request
            .params
            .subject_alt_names
            .iter()
            .map(|san| match san {
                rcgen::SanType::DnsName(name) => name.as_str().to_string(),
                rcgen::SanType::IpAddress(ip) => ip.to_string(),
                other => format!("{other:?}"),
            })
            .collect();
        let mut ordered: Vec<String> = body["identifiers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["value"].as_str().unwrap().to_string())
            .collect();
        asked.sort();
        ordered.sort();
        if asked != ordered {
            return self.problem(
                StatusCode::BAD_REQUEST,
                "badCSR",
                &format!("the request names {asked:?}, the order {ordered:?}"),
            );
        }
        let issuer = Issuer::from_params(&self.issuer.0, &self.issuer.1);
        let leaf = request.signed_by(&issuer).unwrap();
        let chain = format!("{}{}", leaf.pem(), self.ca_pem);
        let mut inner = self.inner.lock().unwrap();
        inner.orders[index].finalized = true;
        inner.orders[index].certificate = Some(chain);
        drop(inner);
        self.order(index)
    }

    fn certificate(&self, index: usize) -> Response {
        let chain = self
            .inner
            .lock()
            .unwrap()
            .orders
            .get(index)
            .and_then(|o| o.certificate.clone());
        match chain {
            Some(chain) => {
                let mut response = self.reply(StatusCode::OK, json!({}), None);
                *response.body_mut() = axum::body::Body::from(chain);
                response.headers_mut().insert(
                    "content-type",
                    HeaderValue::from_static("application/pem-certificate-chain"),
                );
                response
            }
            None => self.problem(StatusCode::NOT_FOUND, "malformed", "no certificate yet"),
        }
    }
}

/// The protected header and payload of a signed request; the signature is
/// not looked at.
fn jws(body: &[u8]) -> (Value, Value) {
    let outer: Value = serde_json::from_slice(body).unwrap_or_default();
    let header = serde_json::from_slice(&unb64(outer["protected"].as_str().unwrap_or_default()))
        .unwrap_or_default();
    let payload = match outer["payload"].as_str() {
        Some("") | None => Value::Null,
        Some(text) => serde_json::from_slice(&unb64(text)).unwrap_or_default(),
    };
    (header, payload)
}
