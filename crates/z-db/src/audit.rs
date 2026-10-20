//! The audit log: what admins did, when, from where.

use sqlx::any::AnyRow;
use sqlx::Row;

use crate::{now, Database, DbError};

/// One thing an admin did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry {
    pub id: i64,
    pub at: i64,
    pub admin_id: Option<i64>,
    pub admin_name: String,
    /// `session`, or `key:<name>` for a request made with an API key.
    pub via: String,
    pub ip: String,
    /// What was done, as `<thing>.<verb>`: `user.create`, `admin.delete`.
    pub action: String,
    /// What it was done to.
    pub target: String,
    pub detail: String,
    /// The HTTP status the request got back.
    pub status: i64,
}

/// What to write down; the id and the time are added.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NewAudit {
    pub admin_id: Option<i64>,
    pub admin_name: String,
    pub via: String,
    pub ip: String,
    pub action: String,
    pub target: String,
    pub detail: String,
    pub status: i64,
}

/// What can narrow the log.
#[derive(Debug, Clone, Default)]
pub struct AuditQuery {
    pub admin_id: Option<i64>,
    /// Entries whose action starts with this.
    pub action: Option<String>,
    /// Part of the admin, action, target or detail.
    pub search: Option<String>,
    /// Only failed requests (status 400 and up).
    pub failed_only: bool,
    /// Entries older than this id, to page back through the log.
    pub before: Option<i64>,
    pub limit: i64,
}

fn entry(row: AnyRow) -> Result<AuditEntry, sqlx::Error> {
    Ok(AuditEntry {
        id: row.try_get("id")?,
        at: row.try_get("at")?,
        admin_id: row.try_get("admin_id")?,
        admin_name: row.try_get("admin_name")?,
        via: row.try_get("via")?,
        ip: row.try_get("ip")?,
        action: row.try_get("action")?,
        target: row.try_get("target")?,
        detail: row.try_get("detail")?,
        status: row.try_get("status")?,
    })
}

impl Database {
    pub async fn add_audit(&self, audit: &NewAudit) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO audit_log (at, admin_id, admin_name, via, ip, action, target, detail, status) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(now())
        .bind(audit.admin_id)
        .bind(&audit.admin_name)
        .bind(&audit.via)
        .bind(&audit.ip)
        .bind(&audit.action)
        .bind(&audit.target)
        .bind(&audit.detail)
        .bind(audit.status)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// The newest entries that pass the filter, newest first.
    pub async fn audit(&self, query: &AuditQuery) -> Result<Vec<AuditEntry>, DbError> {
        let search = query
            .search
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| format!("%{}%", s.to_lowercase()))
            .unwrap_or_default();
        let action = query
            .action
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| format!("{s}%"))
            .unwrap_or_default();
        let rows = sqlx::query(
            "SELECT id, at, admin_id, admin_name, via, ip, action, target, detail, status FROM audit_log \
             WHERE ($1 = 0 OR admin_id = $1) \
             AND ($2 = '' OR action LIKE $2) \
             AND ($3 = '' OR LOWER(admin_name || ' ' || action || ' ' || target || ' ' || detail) LIKE $3) \
             AND ($4 = 0 OR status >= 400) \
             AND ($5 = 0 OR id < $5) \
             ORDER BY id DESC LIMIT $6",
        )
        .bind(query.admin_id.unwrap_or(0))
        .bind(&action)
        .bind(&search)
        .bind(i64::from(query.failed_only))
        .bind(query.before.unwrap_or(0))
        .bind(query.limit.clamp(1, 500))
        .fetch_all(self.pool())
        .await?;
        Ok(rows.into_iter().map(entry).collect::<Result<_, _>>()?)
    }

    /// Forget entries from before `before`; returns how many went.
    pub async fn prune_audit(&self, before: i64) -> Result<u64, DbError> {
        Ok(sqlx::query("DELETE FROM audit_log WHERE at < $1")
            .bind(before)
            .execute(self.pool())
            .await?
            .rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_log_is_written_filtered_paged_and_pruned() {
        let dir = std::env::temp_dir().join(format!("z-db-audit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::open(&format!("sqlite://{}/panel.db", dir.display()))
            .await
            .unwrap();
        let add = |admin: i64, name: &str, action: &str, target: &str, status: i64| {
            let db = db.clone();
            let audit = NewAudit {
                admin_id: Some(admin),
                admin_name: name.into(),
                via: "session".into(),
                ip: "127.0.0.1".into(),
                action: action.into(),
                target: target.into(),
                detail: format!("detail of {target}"),
                status,
            };
            async move { db.add_audit(&audit).await.unwrap() }
        };
        add(1, "owner", "user.create", "alice", 201).await;
        add(1, "owner", "user.delete", "bob", 204).await;
        add(2, "shop", "user.create", "carol", 403).await;
        add(2, "shop", "inbound.update", "vless-1", 200).await;

        let all = db
            .audit(&AuditQuery {
                limit: 50,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(all.len(), 4);
        assert_eq!(all[0].target, "vless-1", "newest first");

        let by = |f: &dyn Fn(&mut AuditQuery)| {
            let mut query = AuditQuery {
                limit: 50,
                ..Default::default()
            };
            f(&mut query);
            let db = db.clone();
            async move { db.audit(&query).await.unwrap() }
        };
        assert_eq!(by(&|q| q.admin_id = Some(2)).await.len(), 2);
        assert_eq!(by(&|q| q.action = Some("user.".into())).await.len(), 3);
        assert_eq!(by(&|q| q.search = Some("ALICE".into())).await.len(), 1);
        assert_eq!(
            by(&|q| q.search = Some("shop inbound".into())).await.len(),
            1
        );
        assert_eq!(by(&|q| q.failed_only = true).await[0].target, "carol");

        let page = by(&|q| q.limit = 2).await;
        assert_eq!(page.len(), 2);
        let older = by(&|q| q.before = Some(page[1].id)).await;
        assert_eq!(
            older.iter().map(|e| e.target.as_str()).collect::<Vec<_>>(),
            ["bob", "alice"]
        );

        assert_eq!(db.prune_audit(0).await.unwrap(), 0);
        assert_eq!(db.prune_audit(i64::MAX).await.unwrap(), 4);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
