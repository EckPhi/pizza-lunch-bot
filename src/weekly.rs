use crate::{ApiError, Call, Result};
use chrono::{DateTime, Datelike, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, io::Write, os::unix::fs::OpenOptionsExt, path::PathBuf};

const HOUR: i64 = 3_600_000;
const DAY: i64 = 24 * HOUR;
const DAYS: [&str; 4] = ["Tuesday", "Wednesday", "Thursday", "Friday"];
const TIMES: [&str; 5] = ["12:00", "12:30", "13:00", "13:30", "14:00"];
pub const STATUSES: [&str; 3] = ["Going", "Maybe", "Can't make it"];

#[derive(Clone, Copy, Deserialize, Serialize)]
pub struct Destination {
    pub chat_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_thread_id: Option<i64>,
}

impl Destination {
    pub fn body(self, mut value: Value) -> Value {
        value["chat_id"] = self.chat_id.into();
        if let Some(thread_id) = self.message_thread_id {
            value["message_thread_id"] = thread_id.into();
        }
        value
    }
}

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Phase {
    Vote,
    Lunch,
    Skip,
    Done,
}

#[derive(Deserialize, Serialize)]
struct Rsvp {
    index: usize,
    name: String,
}

// Match the JavaScript state file, including local-wall-clock epoch milliseconds.
#[derive(Deserialize, Serialize)]
struct Cycle {
    destination: Destination,
    owner: String,
    cutoff: i64,
    phase: Phase,
    votes: BTreeMap<String, Vec<usize>>,
    #[serde(default, rename = "voterNames")]
    voter_names: BTreeMap<String, String>,
    rsvps: BTreeMap<String, Rsvp>,
    time: String,
    dirty: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    day: Option<usize>,
}

impl Cycle {
    fn new(destination: Destination, owner: String, now: DateTime<Utc>) -> Self {
        let weekday = i64::from(now.weekday().num_days_from_monday());
        let today = now.timestamp_millis().div_euclid(DAY) * DAY;
        let monday = today + (7 - weekday) % 7 * DAY + 20 * HOUR;
        let cutoff = if now.timestamp_millis() < monday {
            monday
        } else {
            monday + 7 * DAY
        };
        Self {
            destination,
            owner,
            cutoff,
            phase: Phase::Vote,
            votes: BTreeMap::new(),
            voter_names: BTreeMap::new(),
            rsvps: BTreeMap::new(),
            time: "13:00".into(),
            dirty: true,
            message: None,
            day: None,
        }
    }

    fn totals(&self) -> [usize; 4] {
        std::array::from_fn(|index| {
            self.votes
                .values()
                .filter(|vote| vote.contains(&index))
                .count()
        })
    }

    fn day(&self) -> usize {
        self.day.expect("validated scheduled day")
    }

    fn start_at(&self, time: &str) -> i64 {
        let (hour, minute) = time.split_once(':').expect("validated lunch time");
        let minutes = hour.parse::<i64>().unwrap() * 60 + minute.parse::<i64>().unwrap();
        self.cutoff - 20 * HOUR + (self.day() as i64 + 1) * DAY + minutes * 60_000
    }

    fn date(&self, day: i64) -> String {
        DateTime::from_timestamp_millis(self.cutoff + day * DAY)
            .expect("validated calendar date")
            .format("%-d %b")
            .to_string()
    }

    fn card(&self, zone: Tz) -> Value {
        let mut rows = Vec::new();
        let text = match self.phase {
            Phase::Vote => {
                let totals = self.totals();
                for (index, day) in DAYS.iter().enumerate() {
                    rows.push(vec![button(
                        format!("{day} {} · {}", self.date(index as i64 + 1), totals[index]),
                        "day",
                        index,
                    )]);
                }
                format!(
                    "🍕 Pizza this week?\nTap every day you can make. Tap again to undo.\nVoting closes Monday {} at 20:00 ({zone}).",
                    self.date(0)
                )
            }
            Phase::Skip => "🍕 No pizza meetup this week. Let’s vote for next week.".into(),
            Phase::Lunch | Phase::Done => {
                let index = self.day();
                let day = DAYS[index];
                let mut text = format!(
                    "🍕 Pizza · {day} {} at {}",
                    self.date(index as i64 + 1),
                    self.time
                );
                if self.phase == Phase::Done {
                    text.push_str(" (finished)");
                }
                let unavailable: Vec<_> = self
                    .votes
                    .iter()
                    .filter(|(_, votes)| !votes.is_empty() && !votes.contains(&index))
                    .collect();
                let names = unavailable
                    .iter()
                    .filter_map(|(person, _)| self.voter_names.get(*person))
                    .map(String::as_str);
                append_names(
                    &mut text,
                    &format!("Didn't select {day}"),
                    unavailable.len(),
                    names,
                );
                for (index, status) in STATUSES.iter().enumerate() {
                    let names: Vec<_> = self
                        .rsvps
                        .values()
                        .filter(|rsvp| rsvp.index == index)
                        .map(|rsvp| rsvp.name.as_str())
                        .collect();
                    append_names(&mut text, status, names.len(), names.into_iter());
                }
                if self.phase == Phase::Lunch {
                    text.push_str("\n\nTap to RSVP. The organizer can change the time below. A time change resets RSVPs.");
                    rows.push(
                        STATUSES
                            .iter()
                            .enumerate()
                            .map(|(i, status)| button(*status, "rsvp", i))
                            .collect(),
                    );
                    rows.push(
                        TIMES
                            .iter()
                            .enumerate()
                            .filter(|(_, time)| **time != self.time)
                            .map(|(i, time)| button(*time, "time", i))
                            .collect(),
                    );
                }
                text
            }
        };
        json!({"text": text, "reply_markup": {"inline_keyboard": rows}})
    }

    fn advance(&mut self, now: i64) {
        if self.phase == Phase::Vote && now >= self.cutoff {
            let totals = self.totals();
            let high = totals.iter().copied().max().unwrap_or(0);
            self.day = totals.iter().position(|total| *total == high);
            self.phase = if high == 0 { Phase::Skip } else { Phase::Lunch };
            self.dirty = true;
        }
        if self.phase == Phase::Lunch && now >= self.start_at(&self.time) + HOUR {
            self.phase = Phase::Done;
            self.dirty = true;
        }
    }

    fn select(&mut self, action: &str, index: usize, user: &Value, now: i64) -> (bool, String) {
        let Some(id) = user["id"].as_i64() else {
            return (false, "Missing voter.".into());
        };
        let person = id.to_string();
        match self.phase {
            Phase::Vote if now < self.cutoff && action == "day" && index < DAYS.len() => {
                let choices = self.votes.entry(person.clone()).or_default();
                if let Some(position) = choices.iter().position(|choice| *choice == index) {
                    choices.remove(position);
                } else {
                    choices.push(index);
                }
                self.voter_names.insert(person, display_name(user));
                let response = if choices.is_empty() {
                    "Your votes are cleared.".into()
                } else {
                    let names: Vec<_> = choices.iter().map(|i| DAYS[*i]).collect();
                    format!("Your days: {}", names.join(", "))
                };
                (true, response)
            }
            Phase::Lunch if now < self.start_at(&self.time) + HOUR => match action {
                "rsvp" if index < STATUSES.len() => {
                    self.rsvps.insert(
                        person,
                        Rsvp {
                            index,
                            name: display_name(user),
                        },
                    );
                    (true, "Saved".into())
                }
                "time" if index < TIMES.len() => {
                    let time = TIMES[index];
                    if person != self.owner {
                        (false, "Ask the organizer to change the time.".into())
                    } else if time == self.time {
                        (false, "This time is already selected.".into())
                    } else if now >= self.start_at(time) {
                        (false, "That time has already passed.".into())
                    } else {
                        self.time = time.into();
                        self.rsvps.clear();
                        (true, "Saved".into())
                    }
                }
                _ => (false, "That option is unavailable.".into()),
            },
            _ => (false, "Voting has closed.".into()),
        }
    }
}

fn append_names<'a>(
    text: &mut String,
    label: &str,
    count: usize,
    names: impl Iterator<Item = &'a str>,
) {
    use std::fmt::Write as _;
    let names: Vec<_> = names.filter(|name| !name.is_empty()).take(15).collect();
    let visible = if names.is_empty() {
        "—".into()
    } else {
        names.join(", ")
    };
    let _ = write!(text, "\n{label} ({count}) · {visible}");
    if count > names.len() {
        let _ = write!(text, " +{}", count - names.len());
    }
}

fn display_name(user: &Value) -> String {
    [user["first_name"].as_str(), user["last_name"].as_str()]
        .into_iter()
        .flatten()
        .flat_map(str::split_whitespace)
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(48)
        .collect()
}

fn button(text: impl AsRef<str>, action: &str, index: usize) -> Value {
    json!({"text": text.as_ref(), "callback_data": format!("weekly:{action}:{index}")})
}

pub struct Weekly {
    path: PathBuf,
    zone: Tz,
    cycles: BTreeMap<i64, Cycle>,
}

impl Weekly {
    pub fn load(path: PathBuf, zone: Tz) -> Result<Self> {
        let cycles: BTreeMap<i64, Cycle> = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(error.into()),
        };
        for (key, cycle) in &cycles {
            let valid_day = cycle.day.is_some_and(|day| day < DAYS.len());
            if *key != cycle.destination.chat_id
                || !TIMES.contains(&cycle.time.as_str())
                || (matches!(cycle.phase, Phase::Lunch | Phase::Done) && !valid_day)
                || cycle.votes.values().flatten().any(|i| *i >= DAYS.len())
                || !(-2_208_988_800_000..=32_503_680_000_000).contains(&cycle.cutoff)
            {
                return Err(format!("Invalid weekly state for chat {key}").into());
            }
        }
        Ok(Self { path, zone, cycles })
    }

    fn save(&self) -> Result<()> {
        let mut temp = self.path.as_os_str().to_owned();
        temp.push(".tmp");
        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(&serde_json::to_vec(&self.cycles)?)?;
        file.sync_all()?;
        fs::rename(temp, &self.path)?;
        Ok(())
    }

    fn wall_clock(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        now.with_timezone(&self.zone).naive_local().and_utc()
    }

    pub fn start(
        &mut self,
        destination: Destination,
        owner: String,
        now: DateTime<Utc>,
        call: &Call,
    ) -> Result<()> {
        if self.cycles.contains_key(&destination.chat_id) {
            call(
                "sendMessage",
                destination.body(
                    json!({"text": "🍕 Weekly pizza polls are already running in this group."}),
                ),
            )?;
            return Ok(());
        }
        self.open(destination, owner, self.wall_clock(now), call)
    }

    fn open(
        &mut self,
        destination: Destination,
        owner: String,
        now: DateTime<Utc>,
        call: &Call,
    ) -> Result<()> {
        self.cycles
            .insert(destination.chat_id, Cycle::new(destination, owner, now));
        self.save()?;
        self.render(destination.chat_id, call)
    }

    fn render(&mut self, key: i64, call: &Call) -> Result<()> {
        let cycle = self.cycles.get_mut(&key).expect("known chat");
        if !cycle.dirty {
            return Ok(());
        }
        let mut card = cycle.card(self.zone);
        if let Some(message) = cycle.message {
            card["chat_id"] = cycle.destination.chat_id.into();
            card["message_id"] = message.into();
            if let Err(error) = call("editMessageText", card) {
                let unchanged = error
                    .downcast_ref::<ApiError>()
                    .is_some_and(|api| api.description.contains("message is not modified"));
                if !unchanged {
                    return Err(error);
                }
            }
        } else {
            let message = call("sendMessage", cycle.destination.body(card))?;
            cycle.message = Some(
                message["message_id"]
                    .as_i64()
                    .ok_or("Missing card message ID")?,
            );
        }
        cycle.dirty = false;
        self.save()
    }

    pub fn tick(&mut self, now: DateTime<Utc>, call: &Call) -> Result<()> {
        let now = self.wall_clock(now);
        let keys: Vec<_> = self.cycles.keys().copied().collect();
        for key in keys {
            if let Err(error) = self.tick_chat(key, now, call) {
                eprintln!("Weekly update failed for chat {key}: {error}");
            }
        }
        Ok(())
    }

    fn tick_chat(&mut self, key: i64, now: DateTime<Utc>, call: &Call) -> Result<()> {
        self.cycles
            .get_mut(&key)
            .expect("known chat")
            .advance(now.timestamp_millis());
        if self.cycles[&key].dirty {
            self.save()?;
            self.render(key, call)?;
        }
        let cycle = &self.cycles[&key];
        if matches!(cycle.phase, Phase::Done | Phase::Skip) {
            self.open(cycle.destination, cycle.owner.clone(), now, call)?;
        }
        Ok(())
    }

    pub fn tap(&mut self, query: &Value, now: DateTime<Utc>, call: &Call) -> Result<()> {
        let Some(data) = query["data"]
            .as_str()
            .and_then(|s| s.strip_prefix("weekly:"))
        else {
            return Ok(());
        };
        let answer = |text: &str| {
            call(
                "answerCallbackQuery",
                json!({"callback_query_id": query["id"], "text": text}),
            )
            .map(|_| ())
        };
        let Some((action, index)) = data
            .split_once(':')
            .and_then(|(action, index)| Some((action, index.parse::<usize>().ok()?)))
        else {
            return answer("That option is unavailable.");
        };
        let now = self.wall_clock(now).timestamp_millis();
        let Some(key) = query["message"]["chat"]["id"].as_i64() else {
            return answer("This card is closed.");
        };
        let Some(cycle) = self.cycles.get_mut(&key).filter(|cycle| {
            cycle.message.is_some() && cycle.message == query["message"]["message_id"].as_i64()
        }) else {
            return answer("This card is closed.");
        };
        let (changed, confirmation) = cycle.select(action, index, &query["from"], now);
        if changed {
            cycle.dirty = true;
            self.save()?;
        }
        answer(&confirmation)?;
        if changed {
            self.render(key, call)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, time::SystemTime};

    #[test]
    fn resumes_javascript_poll_and_rolls_to_next_week() -> Result<()> {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_nanos();
        let path = std::env::temp_dir().join(format!("pizza-state-{stamp}.json"));
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = fs::remove_file(&self.0);
            }
        }
        let _cleanup = Cleanup(path.clone());
        let at = |time: &str| time.parse::<DateTime<Utc>>().unwrap();
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "-1": {
                    "destination": {"chat_id": -1, "message_thread_id": 7},
                    "owner": "1",
                    "cutoff": at("2026-11-16T20:00:00Z").timestamp_millis(),
                    "phase": "vote",
                    "votes": {"1":[1,3], "2":[1], "3":[3]},
                    "voterNames": {"1":"Ada", "2":"Ben", "3":"Cora"},
                    "rsvps": {}, "time": "13:00", "dirty": false, "message": 5
                }
            }))?,
        )?;
        let mut weekly = Weekly::load(path.clone(), chrono_tz::Europe::Vienna)?;
        let calls = RefCell::new(Vec::new());
        let call = |method: &str, body: Value| -> Result<Value> {
            calls.borrow_mut().push((method.to_owned(), body));
            Ok(json!({"message_id": 6}))
        };
        let last = || calls.borrow().last().unwrap().clone();
        weekly.tick_chat(-1, weekly.wall_clock(at("2026-11-16T18:59:59Z")), &call)?;
        assert!(calls.borrow().is_empty());
        weekly.tick_chat(-1, weekly.wall_clock(at("2026-11-16T19:00:00Z")), &call)?;
        let (method, card) = last();
        assert_eq!(method, "editMessageText");
        assert_eq!(card["message_id"], 5);
        let text = card["text"].as_str().unwrap();
        assert!(text.contains("Wednesday 18 Nov at 13:00"));
        assert!(text.contains("Didn't select Wednesday (1) · Cora"));

        let tap = |weekly: &mut Weekly, data: &str, user| {
            weekly.tap(
                &json!({
                    "id":"tap", "data":data, "from":{"id":user,"first_name":"Ben"},
                    "message":{"message_id":5,"chat":{"id":-1}}
                }),
                at("2026-11-16T19:01:00Z"),
                &call,
            )
        };
        tap(&mut weekly, "weekly:rsvp:0", 2)?;
        assert!(
            last().1["text"]
                .as_str()
                .unwrap()
                .contains("Going (1) · Ben")
        );
        tap(&mut weekly, "weekly:time:3", 2)?;
        assert_eq!(last().0, "answerCallbackQuery");
        tap(&mut weekly, "weekly:time:3", 1)?;
        let state: Value = serde_json::from_slice(&fs::read(&path)?)?;
        assert_eq!(state["-1"]["time"], "13:30");
        assert_eq!(state["-1"]["rsvps"], json!({}));
        assert_eq!(state["-1"]["voterNames"]["3"], "Cora");

        weekly.tick_chat(-1, weekly.wall_clock(at("2026-11-18T13:30:00Z")), &call)?;
        let (method, poll) = last();
        assert_eq!(method, "sendMessage");
        assert_eq!(poll["message_thread_id"], 7);
        assert!(poll["text"].as_str().unwrap().contains("Monday 23 Nov"));
        Ok(())
    }
}
