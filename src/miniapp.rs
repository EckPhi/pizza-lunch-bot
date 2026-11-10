use crate::{Call, Result, weekly::Weekly};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse},
    routing::{get, post},
};
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::Sha256;
use std::{
    collections::BTreeMap,
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
};
use tokio::sync::Semaphore;

type Signature = Hmac<Sha256>;
type ApiReply = (StatusCode, Json<Value>);

struct App {
    token: String,
    weekly: Arc<Mutex<Weekly>>,
    call: Arc<Call<'static>>,
    slots: Arc<Semaphore>,
}

#[derive(Deserialize)]
struct Selection {
    message: i64,
    action: String,
    index: usize,
}

pub fn serve(
    address: &str,
    token: String,
    weekly: Arc<Mutex<Weekly>>,
    call: Arc<Call<'static>>,
) -> Result<()> {
    let listener = TcpListener::bind(address)?;
    listener.set_nonblocking(true)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let app = Router::new()
        .route("/", get(page))
        .route(
            "/app.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css")],
                    include_str!("miniapp/app.css"),
                )
            }),
        )
        .route(
            "/app.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript")],
                    include_str!("miniapp/app.js"),
                )
            }),
        )
        .route("/api/plan", post(plan))
        .layer(DefaultBodyLimit::max(16_384))
        .with_state(Arc::new(App {
            token,
            weekly,
            call,
            slots: Arc::new(Semaphore::new(12)),
        }));
    thread::spawn(move || {
        let result = runtime.block_on(async {
            axum::serve(tokio::net::TcpListener::from_std(listener)?, app).await
        });
        if let Err(error) = result {
            eprintln!("Mini App server stopped: {error}");
        }
    });
    println!("Pizza Mini App listening on {address}");
    Ok(())
}

async fn page() -> impl IntoResponse {
    (
        [(
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; script-src 'self' https://telegram.org; style-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'",
        )],
        Html(include_str!("miniapp/index.html")),
    )
}

fn failure(status: StatusCode, message: &str) -> ApiReply {
    (status, Json(json!({"error":message})))
}

async fn plan(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(selection): Json<Option<Selection>>,
) -> impl IntoResponse {
    let reply = authenticated_plan(app, headers, selection).await;
    ([(header::CACHE_CONTROL, "no-store")], reply)
}

async fn authenticated_plan(
    app: Arc<App>,
    headers: HeaderMap,
    selection: Option<Selection>,
) -> ApiReply {
    let init_data = headers
        .get(header::AUTHORIZATION)
        .and_then(|header| header.to_str().ok())
        .and_then(|header| header.strip_prefix("tma "));
    let Some((key, user, user_id)) =
        init_data.and_then(|data| validate(data, &app.token, Utc::now().timestamp()))
    else {
        return failure(
            StatusCode::UNAUTHORIZED,
            "Open Pizza Lunch from your Telegram group again.",
        );
    };

    let Ok(permit) = Arc::clone(&app.slots).try_acquire_owned() else {
        return failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "Pizza Lunch is busy. Try again.",
        );
    };

    let response = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let member = (app.call)("getChatMember", json!({"chat_id":key, "user_id":user_id}));
        let member = match member {
            Ok(member) => member,
            Err(_) => return failure(StatusCode::FORBIDDEN, "Pizza Lunch could not verify your group membership. Ask the organizer to check the bot's access."),
        };
        let is_member = matches!(member["status"].as_str(), Some("creator" | "administrator" | "member"))
            || (member["status"] == "restricted" && member["is_member"] == true);
        if !is_member {
            return failure(StatusCode::FORBIDDEN, "This pizza plan is only available to group members.");
        }
        let Ok(mut weekly) = app.weekly.lock() else {
            return failure(StatusCode::INTERNAL_SERVER_ERROR, "Pizza Lunch is temporarily unavailable.");
        };
        let now = Utc::now();
        let notice = if let Some(selection) = selection {
            match weekly.mini_select(key, selection.message, (&selection.action, selection.index), &user, now, &*app.call) {
                Ok(notice) => Some(notice),
                Err(error) => {
                    eprintln!("Mini App selection failed: {error}");
                    return failure(StatusCode::INTERNAL_SERVER_ERROR, "Could not save your choice. Refresh before trying again.");
                }
            }
        } else {
            None
        };
        match weekly.mini_view(key, &user, now) {
            Some(mut view) => {
                view["notice"] = json!(notice);
                (StatusCode::OK, Json(view))
            }
            None => failure(StatusCode::NOT_FOUND, "Start a poll in the group with /weekly first."),
        }
    }).await;

    response.unwrap_or_else(|_| {
        failure(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Pizza Lunch is temporarily unavailable.",
        )
    })
}

fn validate(data: &str, token: &str, now: i64) -> Option<(i64, Value, i64)> {
    let mut fields = BTreeMap::new();
    for (key, value) in url::form_urlencoded::parse(data.as_bytes()) {
        if fields.insert(key, value).is_some() {
            return None;
        }
    }
    let hash = hex::decode(fields.remove("hash")?.as_bytes()).ok()?;
    let check = fields
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut key = Signature::new_from_slice(b"WebAppData").ok()?;
    key.update(token.as_bytes());
    let mut mac = Signature::new_from_slice(&key.finalize().into_bytes()).ok()?;
    mac.update(check.as_bytes());
    mac.verify_slice(&hash).ok()?;

    let issued = fields.get("auth_date")?.parse::<i64>().ok()?;
    if !(now - 3600..=now + 30).contains(&issued)
        || !matches!(fields.get("chat_type")?.as_ref(), "group" | "supergroup")
    {
        return None;
    }
    let chat = fields.get("start_param")?.parse::<i64>().ok()?;
    let user: Value = serde_json::from_str(fields.get("user")?).ok()?;
    let user_id = user["id"].as_i64()?;
    (chat < 0 && user_id > 0).then_some((chat, user, user_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_telegram_signature_and_rejects_tampering() {
        // Independently generated using Python's hmac and urllib.
        let data = "auth_date=1800000000&chat_type=supergroup&start_param=-123&user=%7B%22id%22%3A42%2C%22first_name%22%3A%22Ada%22%7D&signature=example&hash=bc4d618c5443c8886d37cd145f63dbe655c2116dbbfbbc63e32493728c0da872";
        let token = "123:fake-token";
        let (chat, user, id) = validate(data, token, 1_800_000_001).unwrap();
        assert_eq!((chat, id), (-123, 42));
        assert_eq!(user["first_name"], "Ada");
        assert!(validate(data, token, 1_800_003_601).is_none());
        assert!(validate(data, "123:other-token", 1_800_000_001).is_none());
        assert!(validate(&data.replace("-123", "-456"), token, 1_800_000_001).is_none());
        assert!(validate(&format!("{data}&start_param=-123"), token, 1_800_000_001).is_none());
    }
}
