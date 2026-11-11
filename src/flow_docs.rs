use super::*;
use chrono::DateTime;
use std::{
    fmt::Write,
    fs,
    sync::atomic::{AtomicI64, Ordering},
};

fn command(text: &str, user: &Value, chat: &Value) -> Value {
    json!({"message": {
        "text": text, "entities": [{"type": "bot_command", "offset": 0}],
        "chat": chat, "from": user,
    }})
}

fn tap(message: i64, action: &str, user: &Value, chat: &Value) -> Value {
    json!({"callback_query": {
        "id":"example", "data":format!("weekly:{action}"), "from":user,
        "message":{"message_id":message,"chat":chat}
    }})
}

struct Transcript {
    weekly: Weekly,
    calls: Mutex<Vec<(String, Value)>>,
    next_message: AtomicI64,
    markdown: String,
}

impl Transcript {
    fn step(&mut self, label: &str, at: &str, update: Option<Value>) -> Result<()> {
        let now: DateTime<Utc> = at.parse()?;
        let call = |method: &str, body: Value| -> Result<Value> {
            self.calls.lock().unwrap().push((method.into(), body));
            Ok(json!({"message_id": self.next_message.fetch_add(1, Ordering::Relaxed)}))
        };
        if let Some(update) = update {
            handle_at(&update, "pizza_example_bot", &mut self.weekly, &call, now)?;
        } else {
            self.weekly.tick(now, &call)?;
        }
        let calls = self.calls.get_mut().unwrap();
        assert!(!calls.is_empty(), "No bot response for {label}");
        writeln!(self.markdown, "### {label}\n")?;
        for (method, body) in calls.drain(..) {
            writeln!(self.markdown, "**{method}**\n")?;
            let text = body["text"]
                .as_str()
                .or_else(|| body["question"].as_str())
                .unwrap();
            for line in text.lines() {
                writeln!(self.markdown, "> {line}  ")?;
            }
            if let Some(rows) = body["reply_markup"]["inline_keyboard"].as_array() {
                for row in rows.as_slice() {
                    let buttons = row
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|button| {
                            let text = button["text"].as_str().unwrap();
                            button["url"]
                                .as_str()
                                .map_or_else(|| text.to_owned(), |url| format!("[{text}]({url})"))
                        })
                        .collect::<Vec<_>>();
                    if !buttons.is_empty() {
                        writeln!(self.markdown, "> **{}**  ", buttons.join(" · "))?;
                    }
                }
            }
            if let Some(options) = body["options"].as_array() {
                let choices = options
                    .iter()
                    .map(|option| option["text"].as_str().unwrap())
                    .collect::<Vec<_>>();
                let mode = if body["allows_multiple_answers"] == true {
                    "Multiple answers"
                } else {
                    "One answer"
                };
                writeln!(self.markdown, "> {mode} · {}  ", choices.join(" · "))?;
            }
            writeln!(self.markdown)?;
        }
        Ok(())
    }

    fn section(&mut self, title: &str, text: &str) -> Result<()> {
        writeln!(self.markdown, "## {title}\n\n{text}\n")?;
        Ok(())
    }
}

#[test]
fn generate_message_flow_guide() -> Result<()> {
    let scratch = std::env::temp_dir().join(format!("pizza-flow-docs-{}", std::process::id()));
    fs::create_dir(&scratch)?;
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(scratch.clone());
    let mut doc = Transcript {
        weekly: Weekly::load(scratch.join("weekly.json"), chrono_tz::Europe::Vienna)?,
        calls: Mutex::new(Vec::new()),
        next_message: AtomicI64::new(1),
        markdown: "# Pizza Lunch Bot message flows\n\nGenerated from the bot using a simulated Telegram API, sample users and a fixed Europe/Vienna schedule. Replace `pizza_example_bot` with your bot's username.\n\n".into(),
    };
    let ada = json!({"id":1,"first_name":"Ada"});
    let ben = json!({"id":2,"first_name":"Ben"});
    let group = json!({"id":-123,"type":"supergroup","title":"Example pizza group"});
    let private = json!({"id":1,"type":"private"});
    let group_command = |text| Some(command(text, &ada, &group));
    let before = "2026-11-16T18:30:00Z";
    let closed = "2026-11-16T19:00:00Z";

    doc.section("Getting started", "Use `/start` or `/help` to see examples. In groups, include the bot username if Telegram has privacy mode enabled.")?;
    doc.step(
        "`/start` or `/help`",
        before,
        Some(command("/help", &ada, &private)),
    )?;
    doc.step(
        "`/weekly` in a private chat",
        before,
        Some(command("/weekly", &ada, &private)),
    )?;

    doc.section("One-off polls", "Use an availability poll to choose a time, then post an RSVP poll when the date, time, and place are decided. Telegram collects the votes directly.")?;
    doc.step(
        "`/plan Tuesday 12:00 | Wednesday 12:30`",
        before,
        group_command("/plan Tuesday 12:00 | Wednesday 12:30"),
    )?;
    doc.step(
        "`/lunch Wednesday 12:30 at Pizzeria Roma`",
        before,
        group_command("/lunch Wednesday 12:30 at Pizzeria Roma"),
    )?;
    doc.step(
        "`/plan` without enough options",
        before,
        group_command("/plan Tuesday 12:00"),
    )?;
    doc.step(
        "`/lunch` without details and without the Mini App",
        before,
        group_command("/lunch"),
    )?;
    doc.step(
        "`/pizza` without the Mini App",
        before,
        group_command("/pizza"),
    )?;

    doc.section("Mini App forms", "With the Mini App enabled, send either command on its own. In the availability form, enter one time per line. In the RSVP form, enter the date, time, and place. Submitting creates the corresponding one-off poll in the same group or topic. Neither form needs a weekly poll.")?;
    doc.weekly
        .enable_mini_app("https://t.me/pizza_example_bot/pizza".into());
    doc.step(
        "`/plan` opens the availability form",
        before,
        group_command("/plan"),
    )?;
    doc.step(
        "`/lunch` opens the RSVP form",
        before,
        group_command("/lunch"),
    )?;

    doc.section("Weekly voting and status", "The first person to start the cycle becomes its organizer. Days can be toggled independently. At Monday’s cutoff, the most popular day wins and ties go to the earlier day.")?;
    doc.step(
        "`/status` before the cycle starts",
        before,
        group_command("/status"),
    )?;
    let card = doc.next_message.load(Ordering::Relaxed);
    doc.step(
        "`/weekly@pizza_example_bot`",
        before,
        group_command("/weekly@pizza_example_bot"),
    )?;
    doc.step(
        "`/weekly` in a group that already has a cycle",
        before,
        group_command("/weekly"),
    )?;
    doc.step(
        "`/status` with an empty poll",
        before,
        group_command("/status"),
    )?;
    doc.step(
        "`/pizza` opens the weekly Mini App",
        before,
        group_command("/pizza"),
    )?;
    doc.step(
        "Ada selects Wednesday",
        before,
        Some(tap(card, "day:1", &ada, &group)),
    )?;
    doc.step(
        "Ben selects Friday",
        before,
        Some(tap(card, "day:3", &ben, &group)),
    )?;
    doc.step("`/status` during a tie", before, group_command("/status"))?;
    doc.step(
        "Ben clears his vote by tapping Friday again",
        before,
        Some(tap(card, "day:3", &ben, &group)),
    )?;
    doc.step(
        "Ben selects Friday again",
        before,
        Some(tap(card, "day:3", &ben, &group)),
    )?;

    doc.section("Weekly lunch and RSVPs", "After the cutoff, the same card shows the winning date and collects RSVPs. The organizer can change the time, which clears earlier RSVPs. The Mini App shows the same weekly votes and RSVPs.")?;
    doc.step("Monday at 20:00", closed, None)?;
    doc.step(
        "`/status` after the cutoff",
        closed,
        group_command("/status"),
    )?;
    doc.step(
        "Ben RSVPs Going",
        closed,
        Some(tap(card, "rsvp:0", &ben, &group)),
    )?;
    doc.step(
        "Ben tries to change the time",
        closed,
        Some(tap(card, "time:3", &ben, &group)),
    )?;
    doc.step(
        "Ada changes the time to 13:30",
        closed,
        Some(tap(card, "time:3", &ada, &group)),
    )?;
    doc.step("One hour after lunch starts", "2026-11-18T13:30:00Z", None)?;
    doc.step(
        "Ben taps the old RSVP card",
        "2026-11-18T13:31:00Z",
        Some(tap(card, "rsvp:0", &ben, &group)),
    )?;
    doc.step(
        "The next cutoff arrives with no votes",
        "2026-11-23T19:00:00Z",
        None,
    )?;

    doc.section("Access rules", "When an administrator restricts the bot, it rejects blocked commands, button taps, and Mini App requests. Blocking a group also pauses its weekly cycle. Telegram manages votes on native one-off polls.")?;
    doc.weekly.configure_access(true)?;
    doc.weekly
        .access
        .as_mut()
        .unwrap()
        .apply(access::Change::SetRule {
            kind: access::Kind::Group,
            id: "-123".into(),
            rule: access::Rule::Block,
        })?;
    doc.step(
        "`/status` in a blocked group",
        "2026-11-23T19:00:00Z",
        group_command("/status"),
    )?;
    doc.step(
        "Ben taps a weekly button in the blocked group",
        "2026-11-23T19:00:00Z",
        Some(tap(card, "day:1", &ben, &group)),
    )?;
    doc.weekly
        .tick("2026-11-30T19:00:00Z".parse()?, &|method, _| {
            panic!("Blocked groups should not trigger scheduled Telegram calls: {method}")
        })?;

    if let Some(output) = env::var_os("PIZZA_FLOW_DOCS_FILE") {
        fs::write(output, doc.markdown)?;
    }
    Ok(())
}
