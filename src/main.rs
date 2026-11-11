mod miniapp;
mod weekly;

use chrono::Utc;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::{env, error::Error, fmt, path::PathBuf, thread, time::Duration};
use weekly::{Destination, Weekly};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Debug)]
struct ApiError {
    method: String,
    code: i64,
    description: String,
    retry_after: u64,
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} failed (Telegram error {})", self.method, self.code)
    }
}

impl Error for ApiError {}

type Call<'a> = dyn Fn(&str, Value) -> Result<Value> + Send + Sync + 'a;

fn handle(update: &Value, username: &str, weekly: &mut Weekly, call: &Call) -> Result<()> {
    let now = Utc::now();
    if let Some(query) = update.get("callback_query") {
        return weekly.tap(query, now, call);
    }

    let message = &update["message"];
    let Some(text) = message["text"].as_str() else {
        return Ok(());
    };
    if message["entities"][0]["type"] != "bot_command" || message["entities"][0]["offset"] != 0 {
        return Ok(());
    }
    let address = text.split_whitespace().next().unwrap_or_default();
    let (command, recipient) = address.split_once('@').unwrap_or((address, username));
    if !recipient.eq_ignore_ascii_case(username) {
        return Ok(());
    }
    let command = command.to_ascii_lowercase();
    if !matches!(
        command.as_str(),
        "/help" | "/start" | "/weekly" | "/status" | "/plan" | "/lunch" | "/pizza"
    ) {
        return Ok(());
    }

    let destination = Destination {
        chat_id: message["chat"]["id"].as_i64().ok_or("Missing chat ID")?,
        message_thread_id: if message["is_topic_message"] == true {
            message["message_thread_id"].as_i64()
        } else {
            None
        },
    };
    let reply =
        |text: &str| call("sendMessage", destination.body(json!({ "text": text }))).map(|_| ());
    if matches!(command.as_str(), "/help" | "/start") {
        return reply(&format!(
            "🍕 Pizza Bot\n\n/weekly@{username} starts a recurring pizza poll.\n/status@{username} shows the leading day or scheduled lunch.\n\n/plan@{username} Tuesday 12:00 | Wednesday 12:30 | Friday 12:00\nVote for every time you can make.\n\n/lunch@{username} Wednesday 12:30 at your favorite pizzeria\nConfirm the time and place and see who's coming."
        ));
    }
    if !matches!(
        message["chat"]["type"].as_str(),
        Some("group" | "supergroup")
    ) {
        return reply("Add me to your pizza group and use this command there.");
    }
    if command == "/pizza" {
        return weekly.send_mini_app(destination, call);
    }
    if command == "/status" {
        return reply(&weekly.status(destination.chat_id));
    }
    if command == "/weekly" {
        if let Some(owner) = message["from"]["id"]
            .as_i64()
            .filter(|_| message["sender_chat"].is_null())
        {
            return weekly.start(destination, owner.to_string(), now, call);
        }
        return reply("Send this command as yourself to become the organizer.");
    }

    let details = text[address.len()..].trim();
    let planning = command == "/plan";
    if details.is_empty() && weekly.send_poll_form(destination, planning, call)? {
        return Ok(());
    }
    match pizza_poll(planning, details) {
        Ok(body) => call("sendPoll", destination.body(body)).map(|_| ()),
        Err(message) => reply(message),
    }
}

fn pizza_poll(planning: bool, details: &str) -> std::result::Result<Value, &'static str> {
    let (question, choices) = if planning {
        let choices: Vec<_> = details.split('|').map(str::trim).collect();
        if !(2..=12).contains(&choices.len())
            || choices.iter().enumerate().any(|(i, choice)| {
                choice.is_empty()
                    || choice.encode_utf16().count() > 100
                    || choices[..i].contains(choice)
            })
        {
            return Err("Give me 2–12 different times separated by |, up to 100 characters each.");
        }
        (
            "🍕 When can you join for pizza? Select every time that works.".to_owned(),
            choices,
        )
    } else {
        let question = format!("🍕 Pizza · {details}");
        if details.is_empty() || question.encode_utf16().count() > 300 {
            return Err(
                "Add a short date, time and place. Example: /lunch Wednesday 12:30 at your favorite pizzeria",
            );
        }
        (question, weekly::STATUSES.to_vec())
    };
    let options: Vec<_> = choices
        .into_iter()
        .map(|text| json!({ "text": text }))
        .collect();
    Ok(json!({
        "question": question, "options": options, "is_anonymous": false,
        "allows_multiple_answers": planning
    }))
}

fn run() -> Result<()> {
    let token = env::var("TELEGRAM_BOT_TOKEN").unwrap_or_default();
    let token = token.trim().to_owned();
    let valid_token = token.split_once(':').is_some_and(|(id, secret)| {
        !id.is_empty()
            && id.bytes().all(|b| b.is_ascii_digit())
            && !secret.is_empty()
            && secret
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    });
    if !valid_token {
        return Err("Set TELEGRAM_BOT_TOKEN to the token from BotFather.".into());
    }
    let zone = env::var("LUNCH_TIME_ZONE").unwrap_or_else(|_| "Europe/Vienna".into());
    let zone = zone
        .parse()
        .map_err(|_| "LUNCH_TIME_ZONE must be an IANA time zone")?;
    let path = env::var_os("LUNCH_STATE_FILE")
        .map_or_else(|| PathBuf::from("lunch-state.json"), PathBuf::from);
    let mut weekly = Weekly::load(path, zone)?;

    let client = Client::builder().timeout(Duration::from_secs(45)).build()?;
    let api_token = token.clone();
    let call: Arc<Call> = Arc::new(move |method: &str, body: Value| -> Result<Value> {
        let result: Value = client
            .post(format!("https://api.telegram.org/bot{api_token}/{method}"))
            .json(&body)
            .send()
            .and_then(|response| response.json())
            .map_err(|_| format!("{method} could not reach Telegram or read its response"))?;
        if result["ok"] == true {
            return Ok(result["result"].clone());
        }
        Err(Box::new(ApiError {
            method: method.into(),
            code: result["error_code"].as_i64().unwrap_or_default(),
            description: result["description"].as_str().unwrap_or_default().into(),
            retry_after: result["parameters"]["retry_after"].as_u64().unwrap_or(5),
        }))
    });
    let me = call("getMe", json!({}))?;
    let username = me["username"]
        .as_str()
        .ok_or("Telegram returned no bot username")?;
    let mut commands = vec![
        json!({"command":"weekly","description":"Start the weekly pizza poll"}),
        json!({"command":"status","description":"Show the leading day or scheduled pizza lunch"}),
        json!({"command":"plan","description":"Create an availability poll"}),
        json!({"command":"lunch","description":"Create a pizza RSVP poll"}),
        json!({"command":"help","description":"Show pizza bot examples"}),
    ];
    let app = env::var("PIZZA_MINI_APP_SHORT_NAME").unwrap_or_default();
    if !app.is_empty() {
        if !(3..=30).contains(&app.len())
            || !app.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err("Invalid PIZZA_MINI_APP_SHORT_NAME.".into());
        }
        weekly.enable_mini_app(format!("https://t.me/{username}/{app}"));
        commands.insert(
            0,
            json!({"command":"pizza","description":"Open the Pizza Lunch Mini App"}),
        );
    }
    call("setMyCommands", json!({"commands": commands}))?;
    let weekly = Arc::new(Mutex::new(weekly));
    if !app.is_empty() {
        let address = env::var("PIZZA_MINI_APP_LISTEN").unwrap_or_else(|_| "0.0.0.0:8080".into());
        miniapp::serve(&address, token, Arc::clone(&weekly), Arc::clone(&call))?;
    }
    println!("Listening as @{username}. Press Ctrl-C to stop.");
    let mut offset = 0;
    loop {
        if let Err(error) = weekly
            .lock()
            .map_err(|_| "Weekly state lock failed")?
            .tick(Utc::now(), &*call)
        {
            eprintln!("Weekly update failed: {error}");
        }
        let updates = match call(
            "getUpdates",
            json!({
                "offset": offset, "timeout": 25, "allowed_updates": ["message", "callback_query"]
            }),
        ) {
            Ok(updates) => updates,
            Err(error) => {
                let api = error.downcast_ref::<ApiError>();
                if api.is_some_and(|api| matches!(api.code, 401 | 409)) {
                    return Err(format!(
                        "{error}. Check your token, webhook and other running copies."
                    )
                    .into());
                }
                eprintln!("{error}");
                thread::sleep(Duration::from_secs(api.map_or(5, |api| api.retry_after)));
                continue;
            }
        };
        for update in updates
            .as_array()
            .ok_or("Telegram returned an invalid update list")?
        {
            if let Err(error) = handle(
                update,
                username,
                &mut *weekly.lock().map_err(|_| "Weekly state lock failed")?,
                &*call,
            ) {
                eprintln!("{error}");
            }
            offset = update["update_id"].as_i64().ok_or("Missing update ID")? + 1;
        }
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
