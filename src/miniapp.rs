use crate::{
    Call, Result, pizza_poll,
    weekly::{Destination, Weekly},
};
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

#[derive(Deserialize)]
#[serde(untagged)]
enum Request {
    Selection(Selection),
    Poll { details: String },
}

#[derive(Clone, Copy)]
enum Launch {
    Weekly(i64),
    Poll {
        destination: Destination,
        planning: bool,
    },
}

impl Launch {
    fn parse(param: &str) -> Option<Self> {
        if let Some((mode, target)) = param.split_once('_') {
            let planning = match mode {
                "plan" => true,
                "lunch" => false,
                _ => return None,
            };
            let (chat, topic) = match target.split_once('_') {
                Some((chat, topic)) => (chat, Some(topic.parse::<i64>().ok()?)),
                None => (target, None),
            };
            let chat_id = chat.parse().ok()?;
            if chat_id >= 0 || topic.is_some_and(|id| id <= 0) {
                return None;
            }
            Some(Self::Poll {
                destination: Destination {
                    chat_id,
                    message_thread_id: topic,
                },
                planning,
            })
        } else {
            let chat = param.parse().ok()?;
            (chat < 0).then_some(Self::Weekly(chat))
        }
    }
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
    Json(request): Json<Option<Request>>,
) -> impl IntoResponse {
    let reply = authenticated_plan(app, headers, request).await;
    ([(header::CACHE_CONTROL, "no-store")], reply)
}

async fn authenticated_plan(
    app: Arc<App>,
    headers: HeaderMap,
    request: Option<Request>,
) -> ApiReply {
    let init_data = headers
        .get(header::AUTHORIZATION)
        .and_then(|header| header.to_str().ok())
        .and_then(|header| header.strip_prefix("tma "));
    let Some((launch, user, user_id)) =
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
        let key = match launch {
            Launch::Weekly(chat) => chat,
            Launch::Poll { destination, .. } => destination.chat_id,
        };
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
        let selection = match (launch, request) {
            (Launch::Poll { destination, planning }, Some(Request::Poll { details })) => {
                if member["status"] == "restricted" && member["can_send_messages"] != true {
                    return failure(StatusCode::FORBIDDEN, "You need permission to send messages in this group.");
                }
                let body = match pizza_poll(planning, details.trim()) {
                    Ok(body) => body,
                    Err(message) => return failure(StatusCode::BAD_REQUEST, message),
                };
                return match (app.call)("sendPoll", destination.body(body)) {
                    Ok(_) => (StatusCode::OK, Json(json!({"posted": true}))),
                    Err(error) => {
                        eprintln!("Mini App poll submission failed: {error}");
                        failure(StatusCode::BAD_GATEWAY, "Could not confirm the poll was posted. Check the group before trying again.")
                    }
                };
            }
            (Launch::Weekly(_), Some(Request::Selection(selection))) => Some(selection),
            (Launch::Weekly(_), None) => None,
            _ => return failure(StatusCode::BAD_REQUEST, "Open the poll form from the command in your group."),
        };
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

fn validate(data: &str, token: &str, now: i64) -> Option<(Launch, Value, i64)> {
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
    let launch = Launch::parse(fields.get("start_param")?)?;
    let user: Value = serde_json::from_str(fields.get("user")?).ok()?;
    let user_id = user["id"].as_i64()?;
    (user_id > 0).then_some((launch, user, user_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_telegram_signature_and_rejects_tampering() {
        // Independently generated using Python's hmac and urllib.
        let data = "auth_date=1800000000&chat_type=supergroup&start_param=-123&user=%7B%22id%22%3A42%2C%22first_name%22%3A%22Ada%22%7D&signature=example&hash=bc4d618c5443c8886d37cd145f63dbe655c2116dbbfbbc63e32493728c0da872";
        let token = "123:fake-token";
        let (launch, user, id) = validate(data, token, 1_800_000_001).unwrap();
        assert!(matches!(launch, Launch::Weekly(-123)));
        assert_eq!(id, 42);
        assert_eq!(user["first_name"], "Ada");
        assert!(validate(data, token, 1_800_003_601).is_none());
        assert!(validate(data, "123:other-token", 1_800_000_001).is_none());
        assert!(validate(&data.replace("-123", "-456"), token, 1_800_000_001).is_none());
        assert!(validate(&format!("{data}&start_param=-123"), token, 1_800_000_001).is_none());
    }

    #[test]
    fn poll_forms_post_the_expected_poll_to_the_original_topic() -> Result<()> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let calls = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&calls);
        let token = "123:fake-token";
        let app = Arc::new(App {
            token: token.into(),
            weekly: Arc::new(Mutex::new(Weekly::load(
                std::env::temp_dir().join(format!("pizza-form-test-{}.json", std::process::id())),
                chrono_tz::Europe::Vienna,
            )?)),
            call: Arc::new(move |method, body| {
                captured.lock().unwrap().push((method.to_owned(), body));
                Ok(json!({"status":"member"}))
            }),
            slots: Arc::new(Semaphore::new(1)),
        });
        app.weekly
            .lock()
            .unwrap()
            .enable_mini_app("https://t.me/pizza_bot/pizza".into());

        for (command, details, multiple, first_choice) in [
            (
                "/plan",
                "Tuesday 12:00 | Wednesday 12:30",
                true,
                "Tuesday 12:00",
            ),
            ("/lunch", "Wednesday 12:30 at Pizzeria Roma", false, "Going"),
        ] {
            crate::handle(
                &json!({"message": {
                    "text": command, "entities": [{"type":"bot_command", "offset":0}],
                    "chat":{"id":-123, "type":"supergroup"},
                    "is_topic_message":true, "message_thread_id":7
                }}),
                "pizza_bot",
                &mut app.weekly.lock().unwrap(),
                &*app.call,
            )?;
            let link = {
                let calls = calls.lock().unwrap();
                assert_eq!(calls.last().unwrap().0, "sendMessage");
                calls.last().unwrap().1["reply_markup"]["inline_keyboard"][0][0]["url"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            };
            let (_, launch) = link.split_once("?startapp=").unwrap();
            let auth_date = Utc::now().timestamp().to_string();
            let fields = [
                ("auth_date", auth_date.as_str()),
                ("chat_type", "supergroup"),
                ("start_param", launch),
                ("user", r#"{"id":42}"#),
            ];
            let mut key = Signature::new_from_slice(b"WebAppData")?;
            key.update(token.as_bytes());
            let mut mac = Signature::new_from_slice(&key.finalize().into_bytes())?;
            mac.update(
                fields
                    .map(|(key, value)| format!("{key}={value}"))
                    .join("\n")
                    .as_bytes(),
            );
            let data = url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(fields)
                .append_pair("hash", &hex::encode(mac.finalize().into_bytes()))
                .finish();
            let mut headers = HeaderMap::new();
            headers.insert(header::AUTHORIZATION, format!("tma {data}").parse()?);
            let (status, _) = runtime.block_on(authenticated_plan(
                Arc::clone(&app),
                headers,
                Some(Request::Poll {
                    details: details.into(),
                }),
            ));
            assert_eq!(status, StatusCode::OK);
            let calls = calls.lock().unwrap();
            let (method, poll) = calls.last().unwrap();
            assert_eq!(method, "sendPoll");
            assert_eq!(poll["chat_id"], -123);
            assert_eq!(poll["message_thread_id"], 7);
            assert_eq!(poll["allows_multiple_answers"], multiple);
            assert_eq!(poll["options"][0]["text"], first_choice);
        }
        Ok(())
    }
}
