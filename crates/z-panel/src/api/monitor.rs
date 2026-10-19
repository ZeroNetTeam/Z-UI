//! What the dashboard watches: the host's figures, the users' traffic over
//! a stretch of time, and the live log.

use std::convert::Infallible;

use axum::extract::{Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::Json;
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};

use crate::auth::CurrentAdmin;
use crate::error::ApiResult;
use crate::monitor::logs::{Filter, Record};
use crate::monitor::system::{Point, Snapshot};
use crate::server::AppState;

#[derive(Deserialize)]
pub struct SystemQuery {
    /// How far back the history goes; an hour unless asked.
    minutes: Option<i64>,
}

#[derive(Serialize)]
pub struct SystemView {
    /// The latest reading; none until the first one is in.
    now: Option<Snapshot>,
    history: Vec<Point>,
    /// Seconds the panel has been running.
    panel_uptime: u64,
}

pub async fn system(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Query(query): Query<SystemQuery>,
) -> Json<SystemView> {
    let minutes = query.minutes.unwrap_or(60).clamp(1, 24 * 60);
    let monitor = state.monitor();
    Json(SystemView {
        now: monitor.latest(),
        history: monitor.history(minutes, 240),
        panel_uptime: monitor.uptime(),
    })
}

#[derive(Deserialize)]
pub struct TrafficQuery {
    /// How many days back; a week unless asked.
    days: Option<i64>,
    top: Option<i64>,
}

#[derive(Serialize)]
pub struct UsagePoint {
    hour: i64,
    uplink: i64,
    downlink: i64,
}

#[derive(Serialize)]
pub struct TopUser {
    id: i64,
    username: String,
    uplink: i64,
    downlink: i64,
}

#[derive(Serialize)]
pub struct TrafficView {
    /// Every user's bytes added up, hour by hour.
    points: Vec<UsagePoint>,
    top: Vec<TopUser>,
    /// Everything every user has ever moved.
    lifetime: i64,
}

pub async fn traffic(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Query(query): Query<TrafficQuery>,
) -> ApiResult<Json<TrafficView>> {
    let days = query.days.unwrap_or(7).clamp(1, 366);
    let top = query.top.unwrap_or(5).clamp(0, 50);
    let since = z_db::now() - days * 86_400;
    let db = state.db();
    Ok(Json(TrafficView {
        points: db
            .total_usage(since)
            .await?
            .into_iter()
            .map(|p| UsagePoint {
                hour: p.hour,
                uplink: p.uplink,
                downlink: p.downlink,
            })
            .collect(),
        top: db
            .top_usage(since, top)
            .await?
            .into_iter()
            .map(|u| TopUser {
                id: u.user_id,
                username: u.username,
                uplink: u.uplink,
                downlink: u.downlink,
            })
            .collect(),
        lifetime: db.lifetime_total().await?,
    }))
}

#[derive(Deserialize)]
pub struct LogQuery {
    source: Option<String>,
    level: Option<String>,
    search: Option<String>,
    limit: Option<usize>,
}

impl LogQuery {
    fn filter(&self) -> Filter {
        Filter {
            source: self.source.clone(),
            level: self.level.clone(),
            search: self.search.clone(),
        }
    }
}

/// The latest records that pass the filter, oldest first.
pub async fn logs(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Query(query): Query<LogQuery>,
) -> Json<Vec<Record>> {
    let limit = query.limit.unwrap_or(200).clamp(1, 2000);
    Json(
        state
            .monitor()
            .logs
            .recent(&query.filter(), limit)
            .iter()
            .map(|record| (**record).clone())
            .collect(),
    )
}

#[derive(Serialize)]
pub struct LevelView {
    /// `error`, `warn`, `info` or `debug`, or the text of a custom `ZUI_LOG`.
    level: String,
    /// `ZUI_LOG` fixes it: the dashboard cannot change it.
    fixed: bool,
}

pub async fn level(State(state): State<AppState>, _: CurrentAdmin) -> Json<LevelView> {
    let (level, fixed) = state.monitor().logs.level();
    Json(LevelView { level, fixed })
}

#[derive(Deserialize)]
pub struct LevelInput {
    level: String,
}

/// Change how much the panel and the core log, from now on and after a
/// restart.
pub async fn set_level(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Json(input): Json<LevelInput>,
) -> ApiResult<Json<LevelView>> {
    state
        .monitor()
        .logs
        .set_level(&input.level)
        .map_err(|message| crate::error::ApiError::bad_request("invalid_log_level", message))?;
    state
        .db()
        .set_setting(super::LOG_LEVEL_SETTING, &input.level)
        .await?;
    let (level, fixed) = state.monitor().logs.level();
    Ok(Json(LevelView { level, fixed }))
}

fn event(record: &Record) -> Option<Event> {
    Event::default()
        .id(record.id.to_string())
        .json_data(record)
        .ok()
}

/// The backlog, then every new record, as server-sent events. A client that
/// falls too far behind skips ahead rather than slowing the process.
pub async fn stream(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Query(query): Query<LogQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let filter = query.filter();
    let backlog = query.limit.unwrap_or(200).clamp(0, 2000);
    let (past, feed) = state.monitor().logs.watch(&filter, backlog);
    let last_seen = past.last().map_or(0, |record| record.id);
    let past = tokio_stream::iter(past.into_iter().filter_map(|record| event(&record)));
    let live = BroadcastStream::new(feed).filter_map(move |received| {
        let record = received.ok()?;
        (record.id > last_seen && filter.allows(&record))
            .then(|| event(&record))
            .flatten()
    });
    Sse::new(past.chain(live).map(Ok)).keep_alive(KeepAlive::default())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use http_body_util::BodyExt;
    use serde_json::{json, Value};
    use tower::ServiceExt;

    use crate::server::testing::panel;

    /// Read the stream until `wanted` has arrived, adding what comes to `text`.
    async fn read_until(body: &mut Body, text: &mut String, wanted: &str) {
        while !text.contains(wanted) {
            let frame = tokio::time::timeout(Duration::from_secs(5), body.frame())
                .await
                .unwrap_or_else(|_| panic!("no {wanted:?} in {text:?}"))
                .expect("the stream ended")
                .unwrap();
            if let Some(data) = frame.data_ref() {
                text.push_str(&String::from_utf8_lossy(data));
            }
        }
    }

    #[tokio::test]
    async fn only_an_admin_sees_any_of_it() {
        let panel = panel("/").await;
        for path in [
            "/api/monitor/system",
            "/api/monitor/traffic",
            "/api/logs",
            "/api/logs/stream",
        ] {
            assert_eq!(
                panel.get(path, None).await.status,
                StatusCode::UNAUTHORIZED,
                "{path}"
            );
        }
    }

    #[tokio::test]
    async fn the_host_is_reported_with_its_history() {
        let panel = panel("/").await;
        let session = panel.set_up("").await;
        let empty = panel
            .get("/api/monitor/system", Some(&session))
            .await
            .json();
        assert!(
            empty["now"].is_null(),
            "nothing is known before the first reading"
        );
        assert!(empty["history"].as_array().unwrap().is_empty());

        panel.state.monitor().sample();
        panel.state.monitor().sample();
        let view = panel
            .get("/api/monitor/system?minutes=30", Some(&session))
            .await
            .json();
        assert!(view["now"]["cores"].as_u64().unwrap() >= 1, "{view}");
        assert_eq!(view["history"].as_array().unwrap().len(), 2);
        assert!(view["panel_uptime"].is_u64());
        #[cfg(target_os = "linux")]
        {
            assert!(view["now"]["mem_total"].as_u64().unwrap() > 0, "{view}");
            assert!(view["now"]["disk_total"].as_u64().unwrap() > 0, "{view}");
        }
    }

    #[tokio::test]
    async fn traffic_adds_up_users_over_the_range_and_names_the_biggest() {
        let panel = panel("/").await;
        let session = panel.set_up("").await;
        let db = panel.state.db();
        let mut ids = Vec::new();
        for name in ["alice", "bob"] {
            let made = panel
                .send(
                    "POST",
                    "/api/users",
                    Some(&session),
                    Some(json!({"username": name})),
                )
                .await;
            ids.push(made.json()["id"].as_i64().unwrap());
        }
        let delta = |user_id, uplink, downlink| z_db::TrafficDelta {
            user_id,
            uplink,
            downlink,
        };
        db.add_traffic(&[delta(ids[0], 100, 900), delta(ids[1], 10, 40)])
            .await
            .unwrap();

        let view = panel
            .get("/api/monitor/traffic?days=1&top=1", Some(&session))
            .await
            .json();
        let points = view["points"].as_array().unwrap();
        assert_eq!(points.len(), 1, "{view}");
        assert_eq!(
            (points[0]["uplink"].as_i64(), points[0]["downlink"].as_i64()),
            (Some(110), Some(940))
        );
        assert_eq!(view["top"].as_array().unwrap().len(), 1, "the limit holds");
        assert_eq!(view["top"][0]["username"], "alice");
        assert_eq!(view["lifetime"], 1050);
        let wide = panel
            .get("/api/monitor/traffic?days=9999", Some(&session))
            .await;
        assert_eq!(
            wide.status,
            StatusCode::OK,
            "an absurd range is clamped, not refused"
        );
    }

    #[tokio::test]
    async fn the_log_is_filtered_and_streams_live() {
        let panel = panel("/").await;
        let session = panel.set_up("").await;
        let logs = &panel.state.monitor().logs;
        logs.push(1, "info", "zero_runtime::server", "listening on 443".into());
        logs.push(
            2,
            "warn",
            "z_ui::jobs",
            "the enforcement pass failed".into(),
        );
        logs.push(3, "warn", "zero_runtime::server", "handshake failed".into());

        let snapshot = panel
            .get("/api/logs?source=core&level=warn", Some(&session))
            .await
            .json();
        let messages: Vec<&str> = snapshot
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["message"].as_str().unwrap())
            .collect();
        assert_eq!(messages, ["handshake failed"]);
        let found = panel
            .get("/api/logs?search=ENFORCEMENT", Some(&session))
            .await
            .json();
        assert_eq!(found[0]["source"], "panel");
        assert_eq!(found[0]["level"], "warn");

        let request = Request::builder()
            .uri("/api/logs/stream?source=core")
            .header(header::COOKIE, &session)
            .body(Body::empty())
            .unwrap();
        let response = panel.app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers()[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/event-stream"));
        let mut body = response.into_body();
        let mut text = String::new();
        read_until(&mut body, &mut text, "handshake failed").await;
        assert!(
            text.contains("listening on 443"),
            "the backlog comes first: {text}"
        );
        assert!(!text.contains("enforcement"), "the filter holds: {text}");

        logs.push(4, "info", "z_ui::jobs", "not for this watcher".into());
        logs.push(
            5,
            "error",
            "zero_runtime::host",
            "a port could not be bound".into(),
        );
        read_until(&mut body, &mut text, "a port could not be bound").await;
        assert!(!text.contains("not for this watcher"), "{text}");
        let last: Value = serde_json::from_str(
            text.lines()
                .rev()
                .find_map(|l| l.strip_prefix("data:"))
                .unwrap()
                .trim(),
        )
        .unwrap();
        assert_eq!(
            (last["level"].as_str(), last["source"].as_str()),
            (Some("error"), Some("core"))
        );
    }
}
