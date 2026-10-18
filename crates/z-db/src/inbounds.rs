//! Inbounds and the hosts that reach them.

use sqlx::any::AnyRow;
use sqlx::Row;

use crate::{now, Database, DbError};

/// An inbound as stored. `settings`, `stream` and `sniffing` are JSON text
/// in Xray's shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inbound {
    pub id: i64,
    pub tag: String,
    pub remark: String,
    pub enabled: bool,
    pub listen: String,
    pub port: i64,
    pub protocol: String,
    pub settings: String,
    pub stream: String,
    pub sniffing: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// What an inbound is made of, to create or replace one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundFields {
    pub tag: String,
    pub remark: String,
    pub enabled: bool,
    pub listen: String,
    pub port: i64,
    pub protocol: String,
    pub settings: String,
    pub stream: String,
    pub sniffing: String,
}

macro_rules! inbound_columns {
    () => {
        "id, tag, remark, enabled, listen, port, protocol, settings, stream, sniffing, created_at, updated_at"
    };
}

fn inbound(row: AnyRow) -> Result<Inbound, sqlx::Error> {
    Ok(Inbound {
        id: row.try_get("id")?,
        tag: row.try_get("tag")?,
        remark: row.try_get("remark")?,
        enabled: row.try_get::<i64, _>("enabled")? != 0,
        listen: row.try_get("listen")?,
        port: row.try_get("port")?,
        protocol: row.try_get("protocol")?,
        settings: row.try_get("settings")?,
        stream: row.try_get("stream")?,
        sniffing: row.try_get("sniffing")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

/// A host as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Host {
    pub id: i64,
    pub inbound_id: i64,
    pub fields: HostFields,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HostFields {
    pub remark: String,
    pub address: String,
    /// The inbound's own port when absent.
    pub port: Option<i64>,
    pub sni: String,
    pub host: String,
    pub path: String,
    pub alpn: String,
    pub fingerprint: String,
    pub allow_insecure: bool,
    pub enabled: bool,
    pub priority: i64,
}

macro_rules! host_columns {
    () => {
        "id, inbound_id, remark, address, port, sni, host, path, alpn, fingerprint, allow_insecure, enabled, priority, created_at"
    };
}

fn host(row: AnyRow) -> Result<Host, sqlx::Error> {
    Ok(Host {
        id: row.try_get("id")?,
        inbound_id: row.try_get("inbound_id")?,
        fields: HostFields {
            remark: row.try_get("remark")?,
            address: row.try_get("address")?,
            port: row.try_get("port")?,
            sni: row.try_get("sni")?,
            host: row.try_get("host")?,
            path: row.try_get("path")?,
            alpn: row.try_get("alpn")?,
            fingerprint: row.try_get("fingerprint")?,
            allow_insecure: row.try_get::<i64, _>("allow_insecure")? != 0,
            enabled: row.try_get::<i64, _>("enabled")? != 0,
            priority: row.try_get("priority")?,
        },
        created_at: row.try_get("created_at")?,
    })
}

impl Database {
    /// Every inbound, in the order they were made.
    pub async fn inbounds(&self) -> Result<Vec<Inbound>, DbError> {
        let rows = sqlx::query(concat!(
            "SELECT ",
            inbound_columns!(),
            " FROM inbounds ORDER BY id"
        ))
        .fetch_all(self.pool())
        .await?;
        Ok(rows.into_iter().map(inbound).collect::<Result<_, _>>()?)
    }

    pub async fn inbound(&self, id: i64) -> Result<Option<Inbound>, DbError> {
        let row = sqlx::query(concat!(
            "SELECT ",
            inbound_columns!(),
            " FROM inbounds WHERE id = $1"
        ))
        .bind(id)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(inbound).transpose()?)
    }

    pub async fn create_inbound(&self, fields: &InboundFields) -> Result<Inbound, DbError> {
        let now = now();
        let row = sqlx::query(concat!(
            "INSERT INTO inbounds (tag, remark, enabled, listen, port, protocol, settings, stream, sniffing, created_at, updated_at) ",
            "VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $10) RETURNING ",
            inbound_columns!()
        ))
        .bind(&fields.tag)
        .bind(&fields.remark)
        .bind(i64::from(fields.enabled))
        .bind(&fields.listen)
        .bind(fields.port)
        .bind(&fields.protocol)
        .bind(&fields.settings)
        .bind(&fields.stream)
        .bind(&fields.sniffing)
        .bind(now)
        .fetch_one(self.pool())
        .await?;
        Ok(inbound(row)?)
    }

    pub async fn update_inbound(
        &self,
        id: i64,
        fields: &InboundFields,
    ) -> Result<Option<Inbound>, DbError> {
        let row = sqlx::query(concat!(
            "UPDATE inbounds SET tag = $1, remark = $2, enabled = $3, listen = $4, port = $5, ",
            "protocol = $6, settings = $7, stream = $8, sniffing = $9, updated_at = $10 ",
            "WHERE id = $11 RETURNING ",
            inbound_columns!()
        ))
        .bind(&fields.tag)
        .bind(&fields.remark)
        .bind(i64::from(fields.enabled))
        .bind(&fields.listen)
        .bind(fields.port)
        .bind(&fields.protocol)
        .bind(&fields.settings)
        .bind(&fields.stream)
        .bind(&fields.sniffing)
        .bind(now())
        .bind(id)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(inbound).transpose()?)
    }

    /// Delete an inbound and its hosts. Returns whether it existed.
    pub async fn delete_inbound(&self, id: i64) -> Result<bool, DbError> {
        let mut tx = self.pool().begin().await?;
        sqlx::query("DELETE FROM hosts WHERE inbound_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        let deleted = sqlx::query("DELETE FROM inbounds WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        tx.commit().await?;
        Ok(deleted > 0)
    }

    /// The hosts of every inbound, by priority then age.
    pub async fn hosts(&self) -> Result<Vec<Host>, DbError> {
        let rows = sqlx::query(concat!(
            "SELECT ",
            host_columns!(),
            " FROM hosts ORDER BY inbound_id, priority, id"
        ))
        .fetch_all(self.pool())
        .await?;
        Ok(rows.into_iter().map(host).collect::<Result<_, _>>()?)
    }

    pub async fn host(&self, id: i64) -> Result<Option<Host>, DbError> {
        let row = sqlx::query(concat!(
            "SELECT ",
            host_columns!(),
            " FROM hosts WHERE id = $1"
        ))
        .bind(id)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(host).transpose()?)
    }

    pub async fn create_host(&self, inbound_id: i64, fields: &HostFields) -> Result<Host, DbError> {
        let row = sqlx::query(concat!(
            "INSERT INTO hosts (inbound_id, remark, address, port, sni, host, path, alpn, fingerprint, ",
            "allow_insecure, enabled, priority, created_at) ",
            "VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) RETURNING ",
            host_columns!()
        ))
        .bind(inbound_id)
        .bind(&fields.remark)
        .bind(&fields.address)
        .bind(fields.port)
        .bind(&fields.sni)
        .bind(&fields.host)
        .bind(&fields.path)
        .bind(&fields.alpn)
        .bind(&fields.fingerprint)
        .bind(i64::from(fields.allow_insecure))
        .bind(i64::from(fields.enabled))
        .bind(fields.priority)
        .bind(now())
        .fetch_one(self.pool())
        .await?;
        Ok(host(row)?)
    }

    pub async fn update_host(
        &self,
        id: i64,
        inbound_id: i64,
        fields: &HostFields,
    ) -> Result<Option<Host>, DbError> {
        let row = sqlx::query(concat!(
            "UPDATE hosts SET inbound_id = $1, remark = $2, address = $3, port = $4, sni = $5, host = $6, ",
            "path = $7, alpn = $8, fingerprint = $9, allow_insecure = $10, enabled = $11, priority = $12 ",
            "WHERE id = $13 RETURNING ",
            host_columns!()
        ))
        .bind(inbound_id)
        .bind(&fields.remark)
        .bind(&fields.address)
        .bind(fields.port)
        .bind(&fields.sni)
        .bind(&fields.host)
        .bind(&fields.path)
        .bind(&fields.alpn)
        .bind(&fields.fingerprint)
        .bind(i64::from(fields.allow_insecure))
        .bind(i64::from(fields.enabled))
        .bind(fields.priority)
        .bind(id)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(host).transpose()?)
    }

    pub async fn delete_host(&self, id: i64) -> Result<bool, DbError> {
        let deleted = sqlx::query("DELETE FROM hosts WHERE id = $1")
            .bind(id)
            .execute(self.pool())
            .await?
            .rows_affected();
        Ok(deleted > 0)
    }
}
