//! Users, the access groups that give them inbounds, user templates, and
//! hourly usage.

use std::collections::HashMap;

use sqlx::any::AnyRow;
use sqlx::Row;

use crate::{ids_text, now, parse_ids, Database, DbError};

/// A user as stored, with the access groups it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub status: String,
    pub uuid: String,
    pub password: String,
    pub sub_token: String,
    pub data_limit: i64,
    pub used_traffic: i64,
    pub lifetime_traffic: i64,
    pub expire_at: Option<i64>,
    pub on_hold_duration: Option<i64>,
    pub on_hold_timeout: Option<i64>,
    pub reset_strategy: String,
    pub last_reset_at: Option<i64>,
    pub ip_limit: i64,
    pub speed_up: i64,
    pub speed_down: i64,
    pub note: String,
    pub online_at: Option<i64>,
    pub admin_id: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub group_ids: Vec<i64>,
    pub next_plan: Option<NextPlan>,
    /// Devices its subscription may be fetched on; 0 for any.
    pub hwid_limit: i64,
    pub sub_updated_at: Option<i64>,
    pub sub_user_agent: String,
}

/// What a user moves on to when it runs out of data or time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NextPlan {
    /// The new data limit, 0 for none.
    pub data_limit: i64,
    /// Days from the switch, 0 for no date.
    pub expire_days: i64,
    /// Whether the data left of the old limit is added to the new one.
    pub carry_over: bool,
}

/// What an admin sets on a user. Counters, credentials and times the panel
/// keeps itself are not here.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UserFields {
    pub username: String,
    pub status: String,
    pub data_limit: i64,
    pub expire_at: Option<i64>,
    pub on_hold_duration: Option<i64>,
    pub on_hold_timeout: Option<i64>,
    pub reset_strategy: String,
    pub ip_limit: i64,
    pub speed_up: i64,
    pub speed_down: i64,
    pub note: String,
    pub group_ids: Vec<i64>,
    pub next_plan: Option<NextPlan>,
    pub hwid_limit: i64,
}

/// What can narrow a list of users.
#[derive(Debug, Clone, Default)]
pub struct UserQuery {
    /// Part of the username or the note.
    pub search: Option<String>,
    pub status: Option<String>,
    pub group_id: Option<i64>,
    /// Only the users this admin owns.
    pub owner: Option<i64>,
    pub offset: i64,
    pub limit: i64,
}

macro_rules! user_columns {
    () => {
        "id, username, status, uuid, password, sub_token, data_limit, used_traffic, lifetime_traffic, \
         expire_at, on_hold_duration, on_hold_timeout, reset_strategy, last_reset_at, ip_limit, \
         speed_up, speed_down, note, online_at, admin_id, created_at, updated_at, \
         next_data_limit, next_expire_days, next_carry_over, hwid_limit, sub_updated_at, sub_user_agent"
    };
}

fn user(row: AnyRow) -> Result<User, sqlx::Error> {
    Ok(User {
        id: row.try_get("id")?,
        username: row.try_get("username")?,
        status: row.try_get("status")?,
        uuid: row.try_get("uuid")?,
        password: row.try_get("password")?,
        sub_token: row.try_get("sub_token")?,
        data_limit: row.try_get("data_limit")?,
        used_traffic: row.try_get("used_traffic")?,
        lifetime_traffic: row.try_get("lifetime_traffic")?,
        expire_at: row.try_get("expire_at")?,
        on_hold_duration: row.try_get("on_hold_duration")?,
        on_hold_timeout: row.try_get("on_hold_timeout")?,
        reset_strategy: row.try_get("reset_strategy")?,
        last_reset_at: row.try_get("last_reset_at")?,
        ip_limit: row.try_get("ip_limit")?,
        speed_up: row.try_get("speed_up")?,
        speed_down: row.try_get("speed_down")?,
        note: row.try_get("note")?,
        online_at: row.try_get("online_at")?,
        admin_id: row.try_get("admin_id")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        group_ids: Vec::new(),
        next_plan: row
            .try_get::<Option<i64>, _>("next_data_limit")?
            .map(|data_limit| -> Result<NextPlan, sqlx::Error> {
                Ok(NextPlan {
                    data_limit,
                    expire_days: row.try_get("next_expire_days")?,
                    carry_over: row.try_get::<i64, _>("next_carry_over")? != 0,
                })
            })
            .transpose()?,
        hwid_limit: row.try_get("hwid_limit")?,
        sub_updated_at: row.try_get("sub_updated_at")?,
        sub_user_agent: row.try_get("sub_user_agent")?,
    })
}

/// An access group and the inbounds it gives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessGroup {
    pub id: i64,
    pub name: String,
    pub all_inbounds: bool,
    pub inbound_ids: Vec<i64>,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UserTemplate {
    pub id: i64,
    pub name: String,
    pub data_limit: i64,
    pub expire_days: i64,
    pub on_hold: bool,
    pub reset_strategy: String,
    pub ip_limit: i64,
    pub speed_up: i64,
    pub speed_down: i64,
    pub hwid_limit: i64,
    pub group_ids: Vec<i64>,
    pub created_at: i64,
}

macro_rules! template_columns {
    () => {
        "id, name, data_limit, expire_days, on_hold, reset_strategy, ip_limit, speed_up, speed_down, hwid_limit, group_ids, created_at"
    };
}

fn template(row: AnyRow) -> Result<UserTemplate, sqlx::Error> {
    let ids: String = row.try_get("group_ids")?;
    Ok(UserTemplate {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        data_limit: row.try_get("data_limit")?,
        expire_days: row.try_get("expire_days")?,
        on_hold: row.try_get::<i64, _>("on_hold")? != 0,
        reset_strategy: row.try_get("reset_strategy")?,
        ip_limit: row.try_get("ip_limit")?,
        speed_up: row.try_get("speed_up")?,
        speed_down: row.try_get("speed_down")?,
        hwid_limit: row.try_get("hwid_limit")?,
        group_ids: parse_ids(&ids),
        created_at: row.try_get("created_at")?,
    })
}

/// One user's bytes over a stretch of time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserUsage {
    pub user_id: i64,
    pub username: String,
    pub uplink: i64,
    pub downlink: i64,
}

/// A device a user fetched its subscription on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: i64,
    pub user_id: i64,
    pub hwid: String,
    pub platform: String,
    pub os_version: String,
    pub model: String,
    pub user_agent: String,
    pub first_seen: i64,
    pub last_seen: i64,
}

/// What a client says of its device.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeviceFields {
    pub hwid: String,
    pub platform: String,
    pub os_version: String,
    pub model: String,
    pub user_agent: String,
}

/// Bytes one user moved in one hour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsagePoint {
    pub hour: i64,
    pub uplink: i64,
    pub downlink: i64,
}

/// Bytes counted for a user since the last time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrafficDelta {
    pub user_id: i64,
    pub uplink: i64,
    pub downlink: i64,
}

impl Database {
    /// Every user's access groups, by user id.
    async fn user_group_map(&self) -> Result<HashMap<i64, Vec<i64>>, DbError> {
        let rows =
            sqlx::query("SELECT user_id, group_id FROM user_access_groups ORDER BY group_id")
                .fetch_all(self.pool())
                .await?;
        let mut map: HashMap<i64, Vec<i64>> = HashMap::new();
        for row in rows {
            map.entry(row.try_get("user_id")?)
                .or_default()
                .push(row.try_get("group_id")?);
        }
        Ok(map)
    }

    async fn with_groups(&self, mut users: Vec<User>) -> Result<Vec<User>, DbError> {
        let mut map = self.user_group_map().await?;
        for user in &mut users {
            user.group_ids = map.remove(&user.id).unwrap_or_default();
        }
        Ok(users)
    }

    /// `users` (one page of them) with their groups, read for these users only.
    async fn with_groups_of_page(&self, mut users: Vec<User>) -> Result<Vec<User>, DbError> {
        if users.is_empty() {
            return Ok(users);
        }
        // Ids that came out of this database: numbers, so safe to write in.
        let ids = users
            .iter()
            .map(|user| user.id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT user_id, group_id FROM user_access_groups WHERE user_id IN ({ids}) ORDER BY group_id"
        )))
        .fetch_all(self.pool())
        .await?;
        let mut map: HashMap<i64, Vec<i64>> = HashMap::new();
        for row in rows {
            map.entry(row.try_get("user_id")?)
                .or_default()
                .push(row.try_get("group_id")?);
        }
        for user in &mut users {
            user.group_ids = map.remove(&user.id).unwrap_or_default();
        }
        Ok(users)
    }

    /// Every user, with their groups: what the core's configuration and the
    /// enforcement pass read.
    pub async fn all_users(&self) -> Result<Vec<User>, DbError> {
        let rows = sqlx::query(concat!(
            "SELECT ",
            user_columns!(),
            " FROM users ORDER BY id"
        ))
        .fetch_all(self.pool())
        .await?;
        let users = rows.into_iter().map(user).collect::<Result<_, _>>()?;
        self.with_groups(users).await
    }

    /// One page of users, newest first, and how many match in all.
    pub async fn users(&self, query: &UserQuery) -> Result<(Vec<User>, i64), DbError> {
        // Only the filters that were asked for go into the query: a condition
        // that matches everything still makes the database look at every row
        // to count them, which a panel with many users pays on each page.
        enum Arg {
            Text(String),
            Number(i64),
        }
        let mut clauses: Vec<String> = Vec::new();
        let mut args: Vec<Arg> = Vec::new();
        if let Some(search) = query
            .search
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            args.push(Arg::Text(format!("%{}%", search.to_lowercase())));
            let n = args.len();
            clauses.push(format!(
                "(LOWER(username) LIKE ${n} OR LOWER(note) LIKE ${n})"
            ));
        }
        if let Some(status) = query.status.as_deref().filter(|s| !s.is_empty()) {
            args.push(Arg::Text(status.to_string()));
            clauses.push(format!("status = ${}", args.len()));
        }
        if let Some(group) = query.group_id.filter(|g| *g != 0) {
            args.push(Arg::Number(group));
            clauses.push(format!(
                "id IN (SELECT user_id FROM user_access_groups WHERE group_id = ${})",
                args.len()
            ));
        }
        if let Some(owner) = query.owner.filter(|o| *o != 0) {
            args.push(Arg::Number(owner));
            clauses.push(format!("admin_id = ${}", args.len()));
        }
        let filter = if clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", clauses.join(" AND "))
        };
        fn bind<'q>(
            mut statement: sqlx::query::Query<'q, sqlx::Any, sqlx::any::AnyArguments>,
            args: &'q [Arg],
        ) -> sqlx::query::Query<'q, sqlx::Any, sqlx::any::AnyArguments> {
            for arg in args {
                statement = match arg {
                    Arg::Text(text) => statement.bind(text.as_str()),
                    Arg::Number(number) => statement.bind(*number),
                };
            }
            statement
        }
        let total: i64 = bind(
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "SELECT COUNT(*) FROM users{filter}"
            ))),
            &args,
        )
        .fetch_one(self.pool())
        .await?
        .try_get(0)?;
        let n = args.len();
        let rows = bind(
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "SELECT {} FROM users{filter} ORDER BY id DESC LIMIT ${} OFFSET ${}",
                user_columns!(),
                n + 1,
                n + 2
            ))),
            &args,
        )
        .bind(query.limit.clamp(1, 500))
        .bind(query.offset.max(0))
        .fetch_all(self.pool())
        .await?;
        let users = rows.into_iter().map(user).collect::<Result<_, _>>()?;
        Ok((self.with_groups_of_page(users).await?, total))
    }

    async fn one_user(&self, row: Option<AnyRow>) -> Result<Option<User>, DbError> {
        let Some(row) = row else { return Ok(None) };
        let mut user = user(row)?;
        user.group_ids = sqlx::query(
            "SELECT group_id FROM user_access_groups WHERE user_id = $1 ORDER BY group_id",
        )
        .bind(user.id)
        .fetch_all(self.pool())
        .await?
        .into_iter()
        .map(|row| row.try_get(0))
        .collect::<Result<_, _>>()?;
        Ok(Some(user))
    }

    pub async fn user(&self, id: i64) -> Result<Option<User>, DbError> {
        let row = sqlx::query(concat!(
            "SELECT ",
            user_columns!(),
            " FROM users WHERE id = $1"
        ))
        .bind(id)
        .fetch_optional(self.pool())
        .await?;
        self.one_user(row).await
    }

    pub async fn user_by_token(&self, token: &str) -> Result<Option<User>, DbError> {
        let row = sqlx::query(concat!(
            "SELECT ",
            user_columns!(),
            " FROM users WHERE sub_token = $1"
        ))
        .bind(token)
        .fetch_optional(self.pool())
        .await?;
        self.one_user(row).await
    }

    /// Add a user with its credentials. Fails if the username is taken.
    pub async fn create_user(
        &self,
        fields: &UserFields,
        uuid: &str,
        password: &str,
        sub_token: &str,
        admin_id: Option<i64>,
    ) -> Result<User, DbError> {
        let now = now();
        let mut tx = self.pool().begin().await?;
        let row = sqlx::query(concat!(
            "INSERT INTO users (username, status, uuid, password, sub_token, data_limit, expire_at, ",
            "on_hold_duration, on_hold_timeout, reset_strategy, last_reset_at, ip_limit, speed_up, speed_down, ",
            "note, admin_id, created_at, updated_at, next_data_limit, next_expire_days, next_carry_over, hwid_limit) ",
            "VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $11, $11, $17, $18, $19, $20) ",
            "RETURNING id"
        ))
        .bind(&fields.username)
        .bind(&fields.status)
        .bind(uuid)
        .bind(password)
        .bind(sub_token)
        .bind(fields.data_limit)
        .bind(fields.expire_at)
        .bind(fields.on_hold_duration)
        .bind(fields.on_hold_timeout)
        .bind(&fields.reset_strategy)
        .bind(now)
        .bind(fields.ip_limit)
        .bind(fields.speed_up)
        .bind(fields.speed_down)
        .bind(&fields.note)
        .bind(admin_id)
        .bind(fields.next_plan.map(|plan| plan.data_limit))
        .bind(fields.next_plan.map_or(0, |plan| plan.expire_days))
        .bind(i64::from(fields.next_plan.is_some_and(|plan| plan.carry_over)))
        .bind(fields.hwid_limit)
        .fetch_one(&mut *tx)
        .await?;
        let id: i64 = row.try_get(0)?;
        for group in &fields.group_ids {
            sqlx::query("INSERT INTO user_access_groups (user_id, group_id) VALUES ($1, $2)")
                .bind(id)
                .bind(group)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(self.user(id).await?.expect("just created"))
    }

    pub async fn update_user(&self, id: i64, fields: &UserFields) -> Result<Option<User>, DbError> {
        let mut tx = self.pool().begin().await?;
        let changed = sqlx::query(concat!(
            "UPDATE users SET username = $1, status = $2, data_limit = $3, expire_at = $4, ",
            "on_hold_duration = $5, on_hold_timeout = $6, reset_strategy = $7, ip_limit = $8, ",
            "speed_up = $9, speed_down = $10, note = $11, updated_at = $12, next_data_limit = $14, ",
            "next_expire_days = $15, next_carry_over = $16, hwid_limit = $17 WHERE id = $13"
        ))
        .bind(&fields.username)
        .bind(&fields.status)
        .bind(fields.data_limit)
        .bind(fields.expire_at)
        .bind(fields.on_hold_duration)
        .bind(fields.on_hold_timeout)
        .bind(&fields.reset_strategy)
        .bind(fields.ip_limit)
        .bind(fields.speed_up)
        .bind(fields.speed_down)
        .bind(&fields.note)
        .bind(now())
        .bind(id)
        .bind(fields.next_plan.map(|plan| plan.data_limit))
        .bind(fields.next_plan.map_or(0, |plan| plan.expire_days))
        .bind(i64::from(fields.next_plan.is_some_and(|plan| plan.carry_over)))
        .bind(fields.hwid_limit)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if changed == 0 {
            return Ok(None);
        }
        sqlx::query("DELETE FROM user_access_groups WHERE user_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        for group in &fields.group_ids {
            sqlx::query("INSERT INTO user_access_groups (user_id, group_id) VALUES ($1, $2)")
                .bind(id)
                .bind(group)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        self.user(id).await
    }

    pub async fn delete_user(&self, id: i64) -> Result<bool, DbError> {
        let mut tx = self.pool().begin().await?;
        for table in [
            "DELETE FROM user_access_groups WHERE user_id = $1",
            "DELETE FROM user_usage WHERE user_id = $1",
            "DELETE FROM user_devices WHERE user_id = $1",
            "DELETE FROM legacy_users WHERE user_id = $1",
            "DELETE FROM telegram_links WHERE kind = 'user' AND ref_id = $1",
        ] {
            sqlx::query(table).bind(id).execute(&mut *tx).await?;
        }
        let deleted = sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        tx.commit().await?;
        Ok(deleted > 0)
    }

    pub async fn set_user_status(&self, id: i64, status: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE users SET status = $1, updated_at = $2 WHERE id = $3")
            .bind(status)
            .bind(now())
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Start an on-hold user's time: it expires `expire_at` from now on.
    pub async fn start_user_time(&self, id: i64, expire_at: i64) -> Result<(), DbError> {
        sqlx::query(
            "UPDATE users SET status = 'active', expire_at = $1, on_hold_duration = NULL, \
             on_hold_timeout = NULL, updated_at = $2 WHERE id = $3",
        )
        .bind(expire_at)
        .bind(now())
        .bind(id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Move a user on to its next plan: a fresh limit and date, its period's
    /// traffic zeroed, active again, and no plan after it.
    pub async fn apply_next_plan(
        &self,
        id: i64,
        data_limit: i64,
        expire_at: Option<i64>,
    ) -> Result<(), DbError> {
        sqlx::query(
            "UPDATE users SET status = 'active', data_limit = $1, expire_at = $2, used_traffic = 0, \
             last_reset_at = $3, next_data_limit = NULL, next_expire_days = 0, next_carry_over = 0, \
             updated_at = $3 WHERE id = $4",
        )
        .bind(data_limit)
        .bind(expire_at)
        .bind(now())
        .bind(id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Zero the period's traffic, as a reset or a new period does.
    pub async fn reset_user_traffic(&self, id: i64) -> Result<(), DbError> {
        sqlx::query(
            "UPDATE users SET used_traffic = 0, last_reset_at = $1, updated_at = $1 WHERE id = $2",
        )
        .bind(now())
        .bind(id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// New credentials and subscription link: the old ones stop working.
    pub async fn set_user_credentials(
        &self,
        id: i64,
        uuid: &str,
        password: &str,
        sub_token: &str,
    ) -> Result<(), DbError> {
        let mut tx = self.pool().begin().await?;
        sqlx::query("UPDATE users SET uuid = $1, password = $2, sub_token = $3, updated_at = $4 WHERE id = $5")
            .bind(uuid)
            .bind(password)
            .bind(sub_token)
            .bind(now())
            .bind(id)
            .execute(&mut *tx)
            .await?;
        // A link from the panel this user came from is a subscription link too.
        sqlx::query("DELETE FROM legacy_users WHERE user_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Add counted bytes to users and to this hour's usage, in one go.
    pub async fn add_traffic(&self, deltas: &[TrafficDelta]) -> Result<(), DbError> {
        if deltas.is_empty() {
            return Ok(());
        }
        let hour = now() / 3600 * 3600;
        let mut tx = self.pool().begin().await?;
        for delta in deltas {
            let total = delta.uplink + delta.downlink;
            sqlx::query(
                "UPDATE users SET used_traffic = used_traffic + $1, lifetime_traffic = lifetime_traffic + $1 WHERE id = $2",
            )
            .bind(total)
            .bind(delta.user_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO user_usage (user_id, hour, uplink, downlink) VALUES ($1, $2, $3, $4) \
                 ON CONFLICT (user_id, hour) DO UPDATE SET uplink = user_usage.uplink + excluded.uplink, \
                 downlink = user_usage.downlink + excluded.downlink",
            )
            .bind(delta.user_id)
            .bind(hour)
            .bind(delta.uplink)
            .bind(delta.downlink)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn set_users_online(&self, ids: &[i64]) -> Result<(), DbError> {
        let now = now();
        let mut tx = self.pool().begin().await?;
        for id in ids {
            sqlx::query("UPDATE users SET online_at = $1 WHERE id = $2")
                .bind(now)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// A user's hourly usage since `since`, oldest first.
    pub async fn user_usage(&self, id: i64, since: i64) -> Result<Vec<UsagePoint>, DbError> {
        let rows = sqlx::query(
            "SELECT hour, uplink, downlink FROM user_usage WHERE user_id = $1 AND hour >= $2 ORDER BY hour",
        )
        .bind(id)
        .bind(since)
        .fetch_all(self.pool())
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(UsagePoint {
                    hour: row.try_get("hour")?,
                    uplink: row.try_get("uplink")?,
                    downlink: row.try_get("downlink")?,
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(DbError::from)
    }

    /// Every user's bytes added up, hour by hour, since `since`; only the
    /// users `owner` owns, if given.
    pub async fn total_usage(
        &self,
        since: i64,
        owner: Option<i64>,
    ) -> Result<Vec<UsagePoint>, DbError> {
        // A sum of bigints is a numeric in Postgres, which the Any driver
        // cannot read as an integer.
        let rows = sqlx::query(concat!(
            "SELECT hour, CAST(SUM(uplink) AS BIGINT) AS uplink, CAST(SUM(downlink) AS BIGINT) AS downlink ",
            "FROM user_usage WHERE hour >= $1 ",
            "AND ($2 = 0 OR user_id IN (SELECT id FROM users WHERE admin_id = $2)) ",
            "GROUP BY hour ORDER BY hour"
        ))
        .bind(since)
        .bind(owner.unwrap_or(0))
        .fetch_all(self.pool())
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(UsagePoint {
                    hour: row.try_get("hour")?,
                    uplink: row.try_get("uplink")?,
                    downlink: row.try_get("downlink")?,
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(DbError::from)
    }

    /// The users who moved the most since `since`, biggest first; only the
    /// users `owner` owns, if given.
    pub async fn top_usage(
        &self,
        since: i64,
        limit: i64,
        owner: Option<i64>,
    ) -> Result<Vec<UserUsage>, DbError> {
        let rows = sqlx::query(concat!(
            "SELECT u.id AS id, u.username AS username, ",
            "CAST(SUM(w.uplink) AS BIGINT) AS uplink, CAST(SUM(w.downlink) AS BIGINT) AS downlink ",
            "FROM user_usage w JOIN users u ON u.id = w.user_id WHERE w.hour >= $1 ",
            "AND ($3 = 0 OR u.admin_id = $3) ",
            "GROUP BY u.id, u.username ORDER BY SUM(w.uplink) + SUM(w.downlink) DESC LIMIT $2"
        ))
        .bind(since)
        .bind(limit)
        .bind(owner.unwrap_or(0))
        .fetch_all(self.pool())
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(UserUsage {
                    user_id: row.try_get("id")?,
                    username: row.try_get("username")?,
                    uplink: row.try_get("uplink")?,
                    downlink: row.try_get("downlink")?,
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(DbError::from)
    }

    /// Everything every user has ever moved; only the users `owner` owns,
    /// if given.
    pub async fn lifetime_total(&self, owner: Option<i64>) -> Result<i64, DbError> {
        let row = sqlx::query(concat!(
            "SELECT CAST(COALESCE(SUM(lifetime_traffic), 0) AS BIGINT) AS total FROM users ",
            "WHERE ($1 = 0 OR admin_id = $1)"
        ))
        .bind(owner.unwrap_or(0))
        .fetch_one(self.pool())
        .await?;
        Ok(row.try_get("total")?)
    }

    /// Forget hourly usage from before `before`; returns how many rows went.
    /// Users' lifetime totals are kept in the users themselves.
    pub async fn prune_usage(&self, before: i64) -> Result<u64, DbError> {
        Ok(sqlx::query("DELETE FROM user_usage WHERE hour < $1")
            .bind(before)
            .execute(self.pool())
            .await?
            .rows_affected())
    }

    /// How many users there are in each status; only the users `owner`
    /// owns, if given.
    pub async fn user_counts(&self, owner: Option<i64>) -> Result<HashMap<String, i64>, DbError> {
        let rows = sqlx::query(
            "SELECT status, COUNT(*) AS n FROM users WHERE ($1 = 0 OR admin_id = $1) GROUP BY status",
        )
        .bind(owner.unwrap_or(0))
        .fetch_all(self.pool())
        .await?;
        rows.into_iter()
            .map(|row| Ok((row.try_get("status")?, row.try_get("n")?)))
            .collect::<Result<_, sqlx::Error>>()
            .map_err(DbError::from)
    }

    pub async fn access_groups(&self) -> Result<Vec<AccessGroup>, DbError> {
        let groups =
            sqlx::query("SELECT id, name, all_inbounds, created_at FROM access_groups ORDER BY id")
                .fetch_all(self.pool())
                .await?;
        let links = sqlx::query(
            "SELECT group_id, inbound_id FROM access_group_inbounds ORDER BY inbound_id",
        )
        .fetch_all(self.pool())
        .await?;
        let mut inbounds: HashMap<i64, Vec<i64>> = HashMap::new();
        for link in links {
            inbounds
                .entry(link.try_get("group_id")?)
                .or_default()
                .push(link.try_get("inbound_id")?);
        }
        groups
            .into_iter()
            .map(|row| {
                let id: i64 = row.try_get("id")?;
                Ok(AccessGroup {
                    id,
                    name: row.try_get("name")?,
                    all_inbounds: row.try_get::<i64, _>("all_inbounds")? != 0,
                    inbound_ids: inbounds.remove(&id).unwrap_or_default(),
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(DbError::from)
    }

    /// Create (`id` None) or replace an access group.
    pub async fn save_access_group(
        &self,
        id: Option<i64>,
        name: &str,
        all_inbounds: bool,
        inbound_ids: &[i64],
    ) -> Result<i64, DbError> {
        let mut tx = self.pool().begin().await?;
        let id = match id {
            Some(id) => {
                sqlx::query("UPDATE access_groups SET name = $1, all_inbounds = $2 WHERE id = $3")
                    .bind(name)
                    .bind(i64::from(all_inbounds))
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
                id
            }
            None => sqlx::query("INSERT INTO access_groups (name, all_inbounds, created_at) VALUES ($1, $2, $3) RETURNING id")
                .bind(name)
                .bind(i64::from(all_inbounds))
                .bind(now())
                .fetch_one(&mut *tx)
                .await?
                .try_get(0)?,
        };
        sqlx::query("DELETE FROM access_group_inbounds WHERE group_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        for inbound in inbound_ids {
            sqlx::query("INSERT INTO access_group_inbounds (group_id, inbound_id) VALUES ($1, $2)")
                .bind(id)
                .bind(inbound)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(id)
    }

    pub async fn delete_access_group(&self, id: i64) -> Result<bool, DbError> {
        let mut tx = self.pool().begin().await?;
        for statement in [
            "DELETE FROM access_group_inbounds WHERE group_id = $1",
            "DELETE FROM user_access_groups WHERE group_id = $1",
        ] {
            sqlx::query(statement).bind(id).execute(&mut *tx).await?;
        }
        let deleted = sqlx::query("DELETE FROM access_groups WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        tx.commit().await?;
        Ok(deleted > 0)
    }

    /// Forget an inbound in every group, when it is deleted.
    pub async fn forget_inbound_in_groups(&self, inbound_id: i64) -> Result<(), DbError> {
        sqlx::query("DELETE FROM access_group_inbounds WHERE inbound_id = $1")
            .bind(inbound_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn user_templates(&self) -> Result<Vec<UserTemplate>, DbError> {
        let rows = sqlx::query(concat!(
            "SELECT ",
            template_columns!(),
            " FROM user_templates ORDER BY id"
        ))
        .fetch_all(self.pool())
        .await?;
        Ok(rows.into_iter().map(template).collect::<Result<_, _>>()?)
    }

    /// Create (`template.id` 0) or replace a template; returns its id.
    pub async fn save_user_template(&self, template: &UserTemplate) -> Result<i64, DbError> {
        let groups = ids_text(&template.group_ids);
        if template.id == 0 {
            let row = sqlx::query(concat!(
                "INSERT INTO user_templates (name, data_limit, expire_days, on_hold, reset_strategy, ip_limit, ",
                "speed_up, speed_down, group_ids, created_at, hwid_limit) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) RETURNING id"
            ))
            .bind(&template.name)
            .bind(template.data_limit)
            .bind(template.expire_days)
            .bind(i64::from(template.on_hold))
            .bind(&template.reset_strategy)
            .bind(template.ip_limit)
            .bind(template.speed_up)
            .bind(template.speed_down)
            .bind(&groups)
            .bind(now())
            .bind(template.hwid_limit)
            .fetch_one(self.pool())
            .await?;
            return Ok(row.try_get(0)?);
        }
        sqlx::query(concat!(
            "UPDATE user_templates SET name = $1, data_limit = $2, expire_days = $3, on_hold = $4, ",
            "reset_strategy = $5, ip_limit = $6, speed_up = $7, speed_down = $8, group_ids = $9, hwid_limit = $11 WHERE id = $10"
        ))
        .bind(&template.name)
        .bind(template.data_limit)
        .bind(template.expire_days)
        .bind(i64::from(template.on_hold))
        .bind(&template.reset_strategy)
        .bind(template.ip_limit)
        .bind(template.speed_up)
        .bind(template.speed_down)
        .bind(&groups)
        .bind(template.id)
        .bind(template.hwid_limit)
        .execute(self.pool())
        .await?;
        Ok(template.id)
    }

    /// Record a fetch of `id`'s subscription with `user_agent`.
    pub async fn touch_subscription(&self, id: i64, user_agent: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE users SET sub_updated_at = $1, sub_user_agent = $2 WHERE id = $3")
            .bind(now())
            .bind(user_agent)
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn user_devices(&self, user_id: i64) -> Result<Vec<Device>, DbError> {
        let rows = sqlx::query(concat!(
            "SELECT id, user_id, hwid, platform, os_version, model, user_agent, first_seen, last_seen ",
            "FROM user_devices WHERE user_id = $1 ORDER BY first_seen"
        ))
        .bind(user_id)
        .fetch_all(self.pool())
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(Device {
                    id: row.try_get("id")?,
                    user_id: row.try_get("user_id")?,
                    hwid: row.try_get("hwid")?,
                    platform: row.try_get("platform")?,
                    os_version: row.try_get("os_version")?,
                    model: row.try_get("model")?,
                    user_agent: row.try_get("user_agent")?,
                    first_seen: row.try_get("first_seen")?,
                    last_seen: row.try_get("last_seen")?,
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(DbError::from)
    }

    /// Note `device` as seen now: its details refreshed if it is known,
    /// added if not.
    pub async fn save_device(&self, user_id: i64, device: &DeviceFields) -> Result<(), DbError> {
        let now = now();
        let updated = sqlx::query(concat!(
            "UPDATE user_devices SET platform = $1, os_version = $2, model = $3, user_agent = $4, ",
            "last_seen = $5 WHERE user_id = $6 AND hwid = $7"
        ))
        .bind(&device.platform)
        .bind(&device.os_version)
        .bind(&device.model)
        .bind(&device.user_agent)
        .bind(now)
        .bind(user_id)
        .bind(&device.hwid)
        .execute(self.pool())
        .await?
        .rows_affected();
        if updated == 0 {
            sqlx::query(concat!(
                "INSERT INTO user_devices (user_id, hwid, platform, os_version, model, user_agent, ",
                "first_seen, last_seen) VALUES ($1, $2, $3, $4, $5, $6, $7, $7)"
            ))
            .bind(user_id)
            .bind(&device.hwid)
            .bind(&device.platform)
            .bind(&device.os_version)
            .bind(&device.model)
            .bind(&device.user_agent)
            .bind(now)
            .execute(self.pool())
            .await?;
        }
        Ok(())
    }

    /// Forget a device of `user_id` (all of them with `None`).
    pub async fn delete_devices(&self, user_id: i64, device: Option<i64>) -> Result<u64, DbError> {
        let deleted = match device {
            Some(id) => sqlx::query("DELETE FROM user_devices WHERE user_id = $1 AND id = $2")
                .bind(user_id)
                .bind(id),
            None => sqlx::query("DELETE FROM user_devices WHERE user_id = $1").bind(user_id),
        }
        .execute(self.pool())
        .await?
        .rows_affected();
        Ok(deleted)
    }

    pub async fn delete_user_template(&self, id: i64) -> Result<bool, DbError> {
        let deleted = sqlx::query("DELETE FROM user_templates WHERE id = $1")
            .bind(id)
            .execute(self.pool())
            .await?
            .rows_affected();
        Ok(deleted > 0)
    }
}
