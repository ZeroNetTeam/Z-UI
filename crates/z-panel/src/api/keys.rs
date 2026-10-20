//! Keys and ids for the inbound forms, made on the server so the dashboard
//! needs no crypto of its own.

use axum::extract::Query;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use zero_runtime::keys;

use crate::auth::Access;
use crate::error::{ApiError, ApiResult, ErrorBody};

#[derive(Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FromQuery {
    /// An existing private key or seed, to derive its public half.
    private: Option<String>,
}

#[utoipa::path(
    get,
    path = "/keys/x25519",
    operation_id = "keys_x25519",
    tag = "tools",
    summary = "Make an X25519 key pair",
    params(FromQuery),
    responses((status = 200, description = "`private_key` and `public_key`", body = Object), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn x25519(_: Access, Query(query): Query<FromQuery>) -> ApiResult<Json<Value>> {
    let pair = match query.private.filter(|p| !p.is_empty()) {
        Some(private) => keys::x25519_from_private(&private)
            .map_err(|error| ApiError::bad_request("invalid_key", error))?,
        None => keys::x25519(),
    };
    Ok(Json(
        json!({ "private_key": pair.private_key, "public_key": pair.public_key }),
    ))
}

#[utoipa::path(
    get,
    path = "/keys/wireguard",
    operation_id = "keys_wireguard",
    tag = "tools",
    summary = "Make a WireGuard key pair",
    params(FromQuery),
    responses((status = 200, description = "`private_key` and `public_key`", body = Object), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn wireguard(_: Access, Query(query): Query<FromQuery>) -> ApiResult<Json<Value>> {
    let pair = match query.private.filter(|p| !p.is_empty()) {
        Some(private) => keys::wireguard_from_private(&private)
            .map_err(|error| ApiError::bad_request("invalid_key", error))?,
        None => keys::wireguard(),
    };
    Ok(Json(
        json!({ "private_key": pair.private_key, "public_key": pair.public_key }),
    ))
}

#[utoipa::path(
    get,
    path = "/keys/mldsa65",
    operation_id = "keys_mldsa65",
    tag = "tools",
    summary = "Make an ML-DSA-65 key pair",
    params(FromQuery),
    responses((status = 200, description = "The keys", body = Object), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn mldsa65(_: Access, Query(query): Query<FromQuery>) -> ApiResult<Json<Value>> {
    let pair = match query.private.filter(|p| !p.is_empty()) {
        Some(seed) => keys::mldsa65_from_seed(&seed)
            .map_err(|error| ApiError::bad_request("invalid_key", error))?,
        None => keys::mldsa65(),
    };
    Ok(Json(json!({ "seed": pair.seed, "verify": pair.verify })))
}

#[utoipa::path(
    get,
    path = "/keys/uuid",
    operation_id = "keys_uuid",
    tag = "tools",
    summary = "Make a UUID",
    responses((status = 200, description = "`uuid`", body = Object), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn uuid(_: Access) -> Json<Value> {
    Json(json!({ "uuid": keys::uuid() }))
}

#[derive(Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct BytesQuery {
    bytes: Option<usize>,
}

#[utoipa::path(
    get,
    path = "/keys/short-id",
    operation_id = "keys_short_id",
    tag = "tools",
    summary = "Make a REALITY short id",
    params(BytesQuery),
    responses((status = 200, description = "`short_id`", body = Object), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn short_id(_: Access, Query(query): Query<BytesQuery>) -> ApiResult<Json<Value>> {
    let id = keys::short_id(query.bytes.unwrap_or(8))
        .map_err(|error| ApiError::bad_request("invalid_length", error))?;
    Ok(Json(json!({ "short_id": id })))
}

/// A random secret; `bytes` 16 or 32 for a Shadowsocks 2022 key (then in
/// standard base64, as that method wants).
#[utoipa::path(
    get,
    path = "/keys/password",
    operation_id = "keys_password",
    tag = "tools",
    summary = "Make a random password",
    params(BytesQuery),
    responses((status = 200, description = "`password`", body = Object), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn password(_: Access, Query(query): Query<BytesQuery>) -> ApiResult<Json<Value>> {
    let bytes = query.bytes.unwrap_or(16);
    if !(8..=64).contains(&bytes) {
        return Err(ApiError::bad_request("invalid_length", "8 to 64 bytes"));
    }
    // A WireGuard key is 32 random bytes in standard base64, which is also
    // exactly a Shadowsocks 2022 key of that size.
    let secret = if bytes == 32 {
        keys::wireguard().private_key
    } else {
        use rand::RngCore;
        let mut raw = vec![0u8; bytes];
        rand::rngs::OsRng.fill_bytes(&mut raw);
        standard_base64(&raw)
    };
    Ok(Json(json!({ "password": secret })))
}

#[derive(Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct VlessEncryptionQuery {
    #[serde(default)]
    mlkem: bool,
}

#[utoipa::path(
    get,
    path = "/keys/vless-encryption",
    operation_id = "keys_vless_encryption",
    tag = "tools",
    summary = "Make a VLESS encryption key pair",
    params(VlessEncryptionQuery),
    responses((status = 200, description = "The keys", body = Object), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn vless_encryption(_: Access, Query(query): Query<VlessEncryptionQuery>) -> Json<Value> {
    let (encryption, decryption) = keys::vless_encryption(query.mlkem, "native", "0rtt", "600s");
    Json(json!({ "encryption": encryption, "decryption": decryption }))
}

fn standard_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n =
            chunk.iter().fold(0u32, |acc, &b| (acc << 8) | u32::from(b)) << (8 * (3 - chunk.len()));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(super::standard_base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(super::standard_base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(super::standard_base64(b"foob"), "Zm9vYg==");
    }
}
