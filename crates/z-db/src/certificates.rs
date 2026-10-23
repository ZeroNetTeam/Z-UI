//! TLS certificates, shared by the panel and its inbounds.

use sqlx::any::AnyRow;
use sqlx::Row;

use crate::{now, Database, DbError};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Certificate {
    pub id: i64,
    pub name: String,
    /// The names it is valid for, comma-separated.
    pub domains: String,
    pub cert_pem: String,
    pub key_pem: String,
    /// `upload`, `acme` or `self-signed`.
    pub source: String,
    pub not_before: i64,
    pub not_after: i64,
    pub created_at: i64,
    /// Whether the panel renews it before it expires.
    pub auto_renew: bool,
    /// `http-01`, `tls-alpn-01` or `dns-01` for an issued one.
    pub method: String,
    /// `letsencrypt`, `letsencrypt_staging`, `zerossl` or a directory URL.
    pub ca: String,
    /// What the last attempt to issue or renew said; empty when it worked.
    pub last_error: String,
    pub last_attempt_at: i64,
}

macro_rules! certificate_columns {
    () => {
        "id, name, domains, cert_pem, key_pem, source, not_before, not_after, created_at, \
         auto_renew, method, ca, last_error, last_attempt_at"
    };
}

/// A certificate to add.
#[derive(Debug, Clone, Default)]
pub struct NewCertificate {
    pub name: String,
    pub domains: String,
    pub cert_pem: String,
    pub key_pem: String,
    pub source: String,
    pub not_before: i64,
    pub not_after: i64,
    pub auto_renew: bool,
    pub method: String,
    pub ca: String,
}

fn certificate(row: AnyRow) -> Result<Certificate, sqlx::Error> {
    Ok(Certificate {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        domains: row.try_get("domains")?,
        cert_pem: row.try_get("cert_pem")?,
        key_pem: row.try_get("key_pem")?,
        source: row.try_get("source")?,
        not_before: row.try_get("not_before")?,
        not_after: row.try_get("not_after")?,
        created_at: row.try_get("created_at")?,
        auto_renew: row.try_get::<i64, _>("auto_renew")? != 0,
        method: row.try_get("method")?,
        ca: row.try_get("ca")?,
        last_error: row.try_get("last_error")?,
        last_attempt_at: row.try_get("last_attempt_at")?,
    })
}

impl Database {
    pub async fn certificates(&self) -> Result<Vec<Certificate>, DbError> {
        let rows = sqlx::query(concat!(
            "SELECT ",
            certificate_columns!(),
            " FROM certificates ORDER BY id"
        ))
        .fetch_all(self.pool())
        .await?;
        Ok(rows
            .into_iter()
            .map(certificate)
            .collect::<Result<_, _>>()?)
    }

    pub async fn certificate(&self, id: i64) -> Result<Option<Certificate>, DbError> {
        let row = sqlx::query(concat!(
            "SELECT ",
            certificate_columns!(),
            " FROM certificates WHERE id = $1"
        ))
        .bind(id)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(certificate).transpose()?)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_certificate(
        &self,
        name: &str,
        domains: &str,
        cert_pem: &str,
        key_pem: &str,
        source: &str,
        not_before: i64,
        not_after: i64,
    ) -> Result<Certificate, DbError> {
        self.add_certificate(NewCertificate {
            name: name.into(),
            domains: domains.into(),
            cert_pem: cert_pem.into(),
            key_pem: key_pem.into(),
            source: source.into(),
            not_before,
            not_after,
            ..Default::default()
        })
        .await
    }

    pub async fn add_certificate(&self, new: NewCertificate) -> Result<Certificate, DbError> {
        let row = sqlx::query(concat!(
            "INSERT INTO certificates (name, domains, cert_pem, key_pem, source, not_before, not_after, created_at, auto_renew, method, ca) ",
            "VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) RETURNING ",
            certificate_columns!()
        ))
        .bind(new.name)
        .bind(new.domains)
        .bind(new.cert_pem)
        .bind(new.key_pem)
        .bind(new.source)
        .bind(new.not_before)
        .bind(new.not_after)
        .bind(now())
        .bind(i64::from(new.auto_renew))
        .bind(new.method)
        .bind(new.ca)
        .fetch_one(self.pool())
        .await?;
        Ok(certificate(row)?)
    }

    /// Put a renewed certificate in place of the old one, same id, so the
    /// inbounds that use it and the panel's choice follow. A success clears
    /// the last error.
    pub async fn renew_certificate(
        &self,
        id: i64,
        cert_pem: &str,
        key_pem: &str,
        not_before: i64,
        not_after: i64,
    ) -> Result<bool, DbError> {
        let updated = sqlx::query(
            "UPDATE certificates SET cert_pem = $1, key_pem = $2, not_before = $3, not_after = $4, \
             last_error = '', last_attempt_at = $5 WHERE id = $6",
        )
        .bind(cert_pem)
        .bind(key_pem)
        .bind(not_before)
        .bind(not_after)
        .bind(now())
        .bind(id)
        .execute(self.pool())
        .await?
        .rows_affected();
        Ok(updated > 0)
    }

    /// Remember that an attempt to renew failed, and why.
    pub async fn certificate_failed(&self, id: i64, error: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE certificates SET last_error = $1, last_attempt_at = $2 WHERE id = $3")
            .bind(error)
            .bind(now())
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn rename_certificate(&self, id: i64, name: &str) -> Result<bool, DbError> {
        let updated = sqlx::query("UPDATE certificates SET name = $1 WHERE id = $2")
            .bind(name)
            .bind(id)
            .execute(self.pool())
            .await?
            .rows_affected();
        Ok(updated > 0)
    }

    pub async fn set_certificate_auto_renew(&self, id: i64, on: bool) -> Result<bool, DbError> {
        let updated = sqlx::query("UPDATE certificates SET auto_renew = $1 WHERE id = $2")
            .bind(i64::from(on))
            .bind(id)
            .execute(self.pool())
            .await?
            .rows_affected();
        Ok(updated > 0)
    }

    pub async fn delete_certificate(&self, id: i64) -> Result<bool, DbError> {
        let deleted = sqlx::query("DELETE FROM certificates WHERE id = $1")
            .bind(id)
            .execute(self.pool())
            .await?
            .rows_affected();
        Ok(deleted > 0)
    }
}
