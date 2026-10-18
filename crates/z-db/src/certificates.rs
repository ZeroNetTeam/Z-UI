//! TLS certificates, shared by the panel and its inbounds.

use sqlx::any::AnyRow;
use sqlx::Row;

use crate::{now, Database, DbError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Certificate {
    pub id: i64,
    pub name: String,
    /// The names it is valid for, comma-separated.
    pub domains: String,
    pub cert_pem: String,
    pub key_pem: String,
    /// `upload` for now; `acme` and `self-signed` when the panel makes them.
    pub source: String,
    pub not_before: i64,
    pub not_after: i64,
    pub created_at: i64,
}

macro_rules! certificate_columns {
    () => {
        "id, name, domains, cert_pem, key_pem, source, not_before, not_after, created_at"
    };
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
        let row = sqlx::query(concat!(
            "INSERT INTO certificates (name, domains, cert_pem, key_pem, source, not_before, not_after, created_at) ",
            "VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING ",
            certificate_columns!()
        ))
        .bind(name)
        .bind(domains)
        .bind(cert_pem)
        .bind(key_pem)
        .bind(source)
        .bind(not_before)
        .bind(not_after)
        .bind(now())
        .fetch_one(self.pool())
        .await?;
        Ok(certificate(row)?)
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
