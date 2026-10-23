//! The certificate store: PEM certificates and keys the panel and its
//! inbounds present. Uploaded, made self-signed, or got from an authority
//! over ACME and renewed by the panel. Keys never leave the server once
//! stored.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::audit::Audit;
use crate::auth::Access;
use crate::certs::acme::{self, Method};
use crate::certs::{self, inspect, settings as acme_settings};
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::server::AppState;

#[derive(Serialize, utoipa::ToSchema)]
pub struct CertificateView {
    id: i64,
    name: String,
    domains: Vec<String>,
    source: String,
    not_before: i64,
    not_after: i64,
    created_at: i64,
    /// Whether the panel renews it before it expires.
    auto_renew: bool,
    /// The challenge it was issued with: `http-01`, `tls-alpn-01`, `dns-01`.
    method: String,
    /// Who issued it: `letsencrypt`, `zerossl`, a directory URL…
    ca: String,
    /// What the last attempt to renew said; empty when it worked.
    last_error: String,
    last_attempt_at: i64,
    /// When the panel will start renewing it, for one that renews itself.
    renews_at: Option<i64>,
    /// Whether the panel itself serves HTTPS with it.
    panel: bool,
}

impl CertificateView {
    fn of(state: &AppState, c: z_db::Certificate, renew_days: u32) -> Self {
        let renews_at = (c.auto_renew && c.source == "acme")
            .then(|| certs::renew_at(c.not_before, c.not_after, renew_days));
        Self {
            panel: state.panel_certificate() == Some(c.id) && state.tls(),
            id: c.id,
            name: c.name,
            domains: c
                .domains
                .split(',')
                .filter(|d| !d.is_empty())
                .map(str::to_string)
                .collect(),
            source: c.source,
            not_before: c.not_before,
            not_after: c.not_after,
            created_at: c.created_at,
            auto_renew: c.auto_renew,
            method: c.method,
            ca: c.ca,
            last_error: c.last_error,
            last_attempt_at: c.last_attempt_at,
            renews_at,
        }
    }
}

async fn view(state: &AppState, c: z_db::Certificate) -> CertificateView {
    let days = acme_settings::load(state.db()).await.renew_days;
    CertificateView::of(state, c, days)
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct CertificateInput {
    #[serde(default)]
    name: String,
    cert_pem: String,
    key_pem: String,
}

#[utoipa::path(
    get,
    path = "/certificates",
    operation_id = "certificates_list",
    tag = "certificates",
    summary = "List certificates",
    responses((status = 200, description = "The certificates", body = Vec<CertificateView>), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn list(
    State(state): State<AppState>,
    _: Access,
) -> ApiResult<Json<Vec<CertificateView>>> {
    let all = state.db().certificates().await?;
    let days = acme_settings::load(state.db()).await.renew_days;
    Ok(Json(
        all.into_iter()
            .map(|c| CertificateView::of(&state, c, days))
            .collect(),
    ))
}

#[utoipa::path(
    post,
    path = "/certificates",
    operation_id = "certificates_create",
    tag = "certificates",
    summary = "Add a certificate",
    request_body = CertificateInput,
    responses((status = 201, description = "Added", body = CertificateView), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn create(
    State(state): State<AppState>,
    _: Access,
    Json(input): Json<CertificateInput>,
) -> ApiResult<(StatusCode, Audit, Json<CertificateView>)> {
    let parsed = inspect(&input.cert_pem, &input.key_pem)?;
    let name = match input.name.trim() {
        "" => parsed
            .domains
            .first()
            .cloned()
            .unwrap_or_else(|| "certificate".into()),
        name => name.chars().take(128).collect(),
    };
    let certificate = state
        .db()
        .create_certificate(
            &name,
            &parsed.domains.join(","),
            input.cert_pem.trim(),
            input.key_pem.trim(),
            "upload",
            parsed.not_before,
            parsed.not_after,
        )
        .await?;
    let audit = Audit::new("certificate.create", &certificate.name);
    Ok((
        StatusCode::CREATED,
        audit,
        Json(view(&state, certificate).await),
    ))
}

#[utoipa::path(
    delete,
    path = "/certificates/{id}",
    operation_id = "certificates_delete",
    tag = "certificates",
    summary = "Delete a certificate",
    params(("id" = i64, Path, description = "the certificate's id")),
    responses((status = 204, description = "Deleted"), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn delete(
    State(state): State<AppState>,
    _: Access,
    Path(id): Path<i64>,
) -> ApiResult<(StatusCode, Audit, ())> {
    let _guard = state.core_lock().lock().await;
    let name = state.db().certificate(id).await?.map(|c| c.name);
    if state.panel_certificate() == Some(id)
        || crate::panel_settings::load(state.db()).await.certificate_id == Some(id)
    {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "certificate_in_use",
            "the panel serves HTTPS with this certificate; choose another for it first",
        ));
    }
    // An inbound that presents it would no longer build.
    for inbound in state.db().inbounds().await? {
        let stream: serde_json::Value = serde_json::from_str(&inbound.stream).unwrap_or_default();
        if stream["tlsSettings"]["certificateId"].as_i64() == Some(id) {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "certificate_in_use",
                format!("inbound {:?} uses this certificate", inbound.remark),
            ));
        }
    }
    if state.db().delete_certificate(id).await? {
        let audit = Audit::new(
            "certificate.delete",
            name.unwrap_or_else(|| format!("#{id}")),
        );
        Ok((StatusCode::NO_CONTENT, audit, ()))
    } else {
        Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "there is no such certificate",
        ))
    }
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct IssueInput {
    #[serde(default)]
    name: String,
    /// Domain names, `*.domains` and addresses.
    names: Vec<String>,
    #[serde(default)]
    method: Method,
    #[serde(default = "yes")]
    auto_renew: bool,
}

fn yes() -> bool {
    true
}

#[utoipa::path(
    post,
    path = "/certificates/issue",
    operation_id = "certificates_issue",
    tag = "certificates",
    summary = "Get a certificate from a certificate authority",
    description = "Proves the names with an ACME challenge (HTTP on port 80, TLS on 443, or DNS at Cloudflare) and adds the certificate. It can take up to a minute or two. The answer is the stored certificate; its key stays on the server.",
    request_body = IssueInput,
    responses((status = 201, description = "Issued and stored", body = CertificateView), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn issue(
    State(state): State<AppState>,
    _: Access,
    Json(input): Json<IssueInput>,
) -> ApiResult<(StatusCode, Audit, Json<CertificateView>)> {
    let certificate = certs::issue(
        &state,
        certs::IssueRequest {
            name: input.name,
            names: input.names,
            method: input.method,
            auto_renew: input.auto_renew,
        },
    )
    .await?;
    let audit = Audit::new("certificate.issue", &certificate.name)
        .detail(format!("{} · {}", certificate.ca, certificate.method));
    Ok((
        StatusCode::CREATED,
        audit,
        Json(view(&state, certificate).await),
    ))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct SelfSignedInput {
    #[serde(default)]
    name: String,
    names: Vec<String>,
}

#[utoipa::path(
    post,
    path = "/certificates/self-signed",
    operation_id = "certificates_self_signed",
    tag = "certificates",
    summary = "Make a self-signed certificate",
    description = "For use where no authority's certificate will do. Clients do not trust it unless told to; it does not renew.",
    request_body = SelfSignedInput,
    responses((status = 201, description = "Made and stored", body = CertificateView), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn self_signed(
    State(state): State<AppState>,
    _: Access,
    Json(input): Json<SelfSignedInput>,
) -> ApiResult<(StatusCode, Audit, Json<CertificateView>)> {
    let names = acme::parse_names(&input.names)?;
    let (cert_pem, key_pem) = certs::self_signed(&names)?;
    let parsed = inspect(&cert_pem, &key_pem)?;
    let name = match input.name.trim() {
        "" => names[0].text(),
        name => name.chars().take(128).collect(),
    };
    let certificate = state
        .db()
        .add_certificate(z_db::NewCertificate {
            name,
            domains: parsed.domains.join(","),
            cert_pem: cert_pem.trim().to_string(),
            key_pem: key_pem.trim().to_string(),
            source: "self-signed".into(),
            not_before: parsed.not_before,
            not_after: parsed.not_after,
            ..Default::default()
        })
        .await?;
    let audit = Audit::new("certificate.self_signed", &certificate.name);
    Ok((
        StatusCode::CREATED,
        audit,
        Json(view(&state, certificate).await),
    ))
}

#[utoipa::path(
    post,
    path = "/certificates/{id}/renew",
    operation_id = "certificates_renew",
    tag = "certificates",
    summary = "Renew a certificate now",
    description = "Only for one the panel got from an authority. The same names are proven again, the certificate is replaced in place, and the panel and the inbounds that use it take the new one without a restart.",
    params(("id" = i64, Path, description = "the certificate's id")),
    responses((status = 200, description = "Renewed", body = CertificateView), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn renew(
    State(state): State<AppState>,
    _: Access,
    Path(id): Path<i64>,
) -> ApiResult<(Audit, Json<CertificateView>)> {
    let certificate = certs::renew(&state, id).await?;
    let audit = Audit::new("certificate.renew", &certificate.name);
    Ok((audit, Json(view(&state, certificate).await)))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UpdateInput {
    /// A new name, if it changes.
    name: Option<String>,
    /// Whether the panel renews it (only for one from an authority).
    auto_renew: Option<bool>,
}

#[utoipa::path(
    put,
    path = "/certificates/{id}",
    operation_id = "certificates_update",
    tag = "certificates",
    summary = "Rename a certificate, or turn its renewal on or off",
    params(("id" = i64, Path, description = "the certificate's id")),
    request_body = UpdateInput,
    responses((status = 200, description = "Changed", body = CertificateView), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn update(
    State(state): State<AppState>,
    _: Access,
    Path(id): Path<i64>,
    Json(input): Json<UpdateInput>,
) -> ApiResult<(Audit, Json<CertificateView>)> {
    let found = state.db().certificate(id).await?.ok_or_else(|| {
        ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "there is no such certificate",
        )
    })?;
    if let Some(on) = input.auto_renew {
        if on && found.source != "acme" {
            return Err(ApiError::bad_request(
                "not_renewable",
                "only a certificate the panel got from an authority can renew itself",
            ));
        }
        state.db().set_certificate_auto_renew(id, on).await?;
    }
    if let Some(name) = input.name.as_deref().map(str::trim) {
        if name.is_empty() {
            return Err(ApiError::bad_request(
                "invalid_name",
                "a certificate needs a name",
            ));
        }
        state
            .db()
            .rename_certificate(id, &name.chars().take(128).collect::<String>())
            .await?;
    }
    let changed = state.db().certificate(id).await?.unwrap_or(found);
    let audit = Audit::new("certificate.update", &changed.name);
    Ok((audit, Json(view(&state, changed).await)))
}

#[utoipa::path(
    get,
    path = "/certificates/acme",
    operation_id = "certificates_acme_get",
    tag = "certificates",
    summary = "Read the settings for getting certificates",
    responses((status = 200, description = "Which authority, the email, the ports; no secret is in it", body = acme_settings::View), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn acme_get(State(state): State<AppState>, _: Access) -> Json<acme_settings::View> {
    Json(acme_settings::load(state.db()).await.view())
}

#[utoipa::path(
    put,
    path = "/certificates/acme",
    operation_id = "certificates_acme_save",
    tag = "certificates",
    summary = "Change the settings for getting certificates",
    request_body = acme_settings::Input,
    responses((status = 200, description = "Saved. A secret left empty is kept as it was", body = acme_settings::View), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn acme_save(
    State(state): State<AppState>,
    _: Access,
    Json(input): Json<acme_settings::Input>,
) -> ApiResult<(Audit, Json<acme_settings::View>)> {
    let next = acme_settings::load(state.db())
        .await
        .accept(input)
        .map_err(|message| ApiError::bad_request("invalid_acme", message))?;
    acme_settings::save(state.db(), &next).await?;
    Ok((Audit::new("certificate.settings", ""), Json(next.view())))
}
