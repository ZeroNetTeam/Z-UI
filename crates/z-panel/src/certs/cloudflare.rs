//! Cloudflare's API, as far as a DNS challenge needs it: find the zone a
//! name lives in, put a TXT record there, take it away again.

use std::time::Duration;

use serde_json::{json, Value};

use crate::notify::http::Proxy;
use crate::outbound::{self, Request};

pub struct Cloudflare {
    pub token: String,
    /// `https://api.cloudflare.com/client/v4`.
    pub base: String,
    pub proxy: Option<Proxy>,
}

/// A record that was written, to be removed afterwards.
#[derive(Debug, Clone)]
pub struct Record {
    pub zone: String,
    pub id: String,
}

impl Cloudflare {
    async fn call(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
        let url = format!("{}{path}", self.base.trim_end_matches('/'));
        let bearer = format!("Bearer {}", self.token);
        let body = body.map(|value| value.to_string());
        let reply = outbound::send(Request {
            method,
            url: &url,
            headers: &[
                ("Authorization", bearer.as_str()),
                ("Accept", "application/json"),
            ],
            body: body
                .as_deref()
                .map(|text| ("application/json", text.as_bytes())),
            proxy: self.proxy.as_ref(),
            timeout: Duration::from_secs(30),
            max_bytes: outbound::DEFAULT_MAX,
            redirects: 0,
        })
        .await
        .map_err(|error| format!("Cloudflare could not be reached: {error}"))?;
        let json: Value = serde_json::from_slice(&reply.body).unwrap_or(Value::Null);
        if reply.is_success() && json["success"].as_bool().unwrap_or(false) {
            return Ok(json["result"].clone());
        }
        let said = json["errors"]
            .as_array()
            .map(|errors| {
                errors
                    .iter()
                    .filter_map(|e| e["message"].as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| format!("HTTP {}", reply.status));
        Err(match reply.status {
            401 | 403 => format!(
                "Cloudflare refused the token ({said}); it needs the permission Zone → DNS → Edit"
            ),
            _ => format!("Cloudflare said: {said}"),
        })
    }

    /// The id of the zone `name` is in: the longest parent that is one.
    pub async fn zone_for(&self, name: &str) -> Result<String, String> {
        let name = name.trim_end_matches('.').to_ascii_lowercase();
        let labels: Vec<&str> = name.split('.').collect();
        for start in 0..labels.len().saturating_sub(1) {
            let candidate = labels[start..].join(".");
            let query = percent_encoding::utf8_percent_encode(
                &candidate,
                percent_encoding::NON_ALPHANUMERIC,
            );
            let found = self
                .call("GET", &format!("/zones?name={query}&status=active"), None)
                .await?;
            if let Some(id) = found
                .as_array()
                .and_then(|zones| zones.first())
                .and_then(|zone| zone["id"].as_str())
            {
                return Ok(id.to_string());
            }
        }
        Err(format!(
            "no zone of this Cloudflare account holds {name}; is the domain added there, and does the token cover it?"
        ))
    }

    /// Write a TXT record `name` with `value`.
    pub async fn add_txt(&self, name: &str, value: &str) -> Result<Record, String> {
        let zone = self.zone_for(name).await?;
        let made = self
            .call(
                "POST",
                &format!("/zones/{zone}/dns_records"),
                Some(json!({"type": "TXT", "name": name, "content": value, "ttl": 60})),
            )
            .await?;
        let id = made["id"]
            .as_str()
            .ok_or("Cloudflare did not say what the record's id is")?
            .to_string();
        Ok(Record { zone, id })
    }

    /// Remove a record written before; a failure is only worth a log line.
    pub async fn remove(&self, record: &Record) {
        let path = format!("/zones/{}/dns_records/{}", record.zone, record.id);
        if let Err(error) = self.call("DELETE", &path, None).await {
            tracing::warn!(%error, "a DNS challenge record could not be removed");
        }
    }
}
