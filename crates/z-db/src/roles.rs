//! Roles: named sets of permissions an admin can be given.

use sqlx::any::AnyRow;
use sqlx::Row;

use crate::{now, Database, DbError};

/// A role. `permissions` is the comma-separated text it is stored as; what
/// the entries mean is the panel's business, not the database's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Role {
    pub id: i64,
    pub name: String,
    pub permissions: String,
    /// Whoever has it sees and changes only the users they made.
    pub own_users_only: bool,
    pub created_at: i64,
}

/// The roles a fresh panel starts with, to be edited or removed.
pub const DEFAULT_ROLES: [(&str, &str, bool); 3] = [
    (
        "Reseller",
        "users.write,groups.read,templates.read,api.write",
        true,
    ),
    (
        "Operator",
        "users.write,groups.read,templates.write,inbounds.read,hosts.write,certificates.read,core.read,monitor.read,logs.read,settings.read",
        false,
    ),
    (
        "Viewer",
        "users.read,groups.read,templates.read,inbounds.read,hosts.read,certificates.read,core.read,monitor.read,logs.read,settings.read,audit.read",
        false,
    ),
];

/// The setting that remembers the default roles were made, so deleting them
/// is not undone at the next start.
const SEEDED: &str = "roles.seeded";

fn role(row: AnyRow) -> Result<Role, sqlx::Error> {
    Ok(Role {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        permissions: row.try_get("permissions")?,
        own_users_only: row.try_get::<i64, _>("own_users_only")? != 0,
        created_at: row.try_get("created_at")?,
    })
}

impl Database {
    pub async fn roles(&self) -> Result<Vec<Role>, DbError> {
        let rows = sqlx::query(
            "SELECT id, name, permissions, own_users_only, created_at FROM roles ORDER BY id",
        )
        .fetch_all(self.pool())
        .await?;
        Ok(rows.into_iter().map(role).collect::<Result<_, _>>()?)
    }

    pub async fn role(&self, id: i64) -> Result<Option<Role>, DbError> {
        let row = sqlx::query(
            "SELECT id, name, permissions, own_users_only, created_at FROM roles WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(role).transpose()?)
    }

    /// Add a role. Fails if the name is taken.
    pub async fn create_role(
        &self,
        name: &str,
        permissions: &str,
        own_users_only: bool,
    ) -> Result<Role, DbError> {
        let row = sqlx::query(
            "INSERT INTO roles (name, permissions, own_users_only, created_at) \
             VALUES ($1, $2, $3, $4) RETURNING id, name, permissions, own_users_only, created_at",
        )
        .bind(name)
        .bind(permissions)
        .bind(i64::from(own_users_only))
        .bind(now())
        .fetch_one(self.pool())
        .await?;
        Ok(role(row)?)
    }

    pub async fn update_role(
        &self,
        id: i64,
        name: &str,
        permissions: &str,
        own_users_only: bool,
    ) -> Result<Option<Role>, DbError> {
        let row = sqlx::query(
            "UPDATE roles SET name = $1, permissions = $2, own_users_only = $3 WHERE id = $4 \
             RETURNING id, name, permissions, own_users_only, created_at",
        )
        .bind(name)
        .bind(permissions)
        .bind(i64::from(own_users_only))
        .bind(id)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(role).transpose()?)
    }

    pub async fn delete_role(&self, id: i64) -> Result<bool, DbError> {
        Ok(sqlx::query("DELETE FROM roles WHERE id = $1")
            .bind(id)
            .execute(self.pool())
            .await?
            .rows_affected()
            > 0)
    }

    /// Make the default roles, once in the life of a panel.
    pub(crate) async fn seed_roles(&self) -> Result<(), DbError> {
        if self.setting(SEEDED).await?.is_some() {
            return Ok(());
        }
        if self.roles().await?.is_empty() {
            for (name, permissions, own_users_only) in DEFAULT_ROLES {
                self.create_role(name, permissions, own_users_only).await?;
            }
        }
        self.set_setting(SEEDED, "1").await
    }
}

#[cfg(test)]
mod tests {
    use crate::Database;

    #[tokio::test]
    async fn a_fresh_panel_has_the_default_roles_and_only_once() {
        let dir = std::env::temp_dir().join(format!("z-db-roles-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let url = format!("sqlite://{}/panel.db", dir.display());
        let db = Database::open(&url).await.unwrap();
        let roles = db.roles().await.unwrap();
        assert_eq!(
            roles.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["Reseller", "Operator", "Viewer"]
        );
        assert!(roles[0].own_users_only && !roles[1].own_users_only);

        let shop = db
            .update_role(roles[0].id, "Shop", "users.read", true)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (shop.name.as_str(), shop.permissions.as_str()),
            ("Shop", "users.read")
        );
        assert!(
            db.create_role("Shop", "", false).await.is_err(),
            "names are unique"
        );
        assert!(db.update_role(999, "x", "", false).await.unwrap().is_none());
        for role in db.roles().await.unwrap() {
            assert!(db.delete_role(role.id).await.unwrap());
        }
        drop(db);

        // Deleted on purpose: a restart does not bring them back.
        let db = Database::open(&url).await.unwrap();
        assert!(db.roles().await.unwrap().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
