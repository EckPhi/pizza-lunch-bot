import { readFile, rename, writeFile } from "node:fs/promises";

const dayMs = 86_400_000;
const days = ["Tuesday", "Wednesday", "Thursday", "Friday"];
const times = ["12:00", "12:30", "13:00", "13:30", "14:00"];
const statuses = ["Going", "Maybe", "Can't make it"];

function button(text, action, index) {
  return { text, callback_data: ["weekly", action, index].join(":") };
}

function displayName(user) {
  return [user.first_name, user.last_name].filter(Boolean).join(" ")
    .replace(/\s+/g, " ").slice(0, 48);
}

export function weeklyLunches(call, file, timeZone, clock = () => new Date()) {
  const formatter = new Intl.DateTimeFormat("en-GB", {
    timeZone, year: "numeric", month: "numeric", day: "numeric",
    hour: "numeric", minute: "numeric", hourCycle: "h23",
  });
  let cycles = {};

  // Compare calendar dates in the group's time zone, including across DST.
  function localTime() {
    const p = Object.fromEntries(formatter.formatToParts(clock()).map(({type, value}) => [type, Number(value)]));
    return Date.UTC(p.year, p.month - 1, p.day, p.hour, p.minute);
  }

  function cutoff(now) {
    const date = new Date(now);
    const today = Date.UTC(date.getUTCFullYear(), date.getUTCMonth(), date.getUTCDate());
    const monday = today + ((8 - date.getUTCDay()) % 7) * dayMs + 20 * 3_600_000;
    return now < monday ? monday : monday + 7 * dayMs;
  }

  function dateLabel(timestamp) {
    return new Intl.DateTimeFormat("en-GB", {
      timeZone: "UTC", day: "numeric", month: "short",
    }).format(timestamp);
  }

  async function save() {
    await writeFile(file + ".tmp", JSON.stringify(cycles), { mode: 0o600 });
    await rename(file + ".tmp", file);
  }

  function counts(cycle) {
    return days.map((_, index) =>
      Object.values(cycle.votes).filter((choices) => choices.includes(index)).length);
  }

  function lunchEnd(cycle) {
    const [hour, minute] = cycle.time.split(":").map(Number);
    return cycle.cutoff - 20 * 3_600_000 +
      (cycle.day + 1) * dayMs + ((hour + 1) * 60 + minute) * 60_000;
  }

  function card(cycle) {
    let text;
    let rows = [];
    if (cycle.phase === "vote") {
      const totals = counts(cycle);
      text = "🍕 Pizza this week?\nTap every day you can make. Tap again to undo.\nVoting closes Monday " +
        dateLabel(cycle.cutoff) + " at 20:00 (" + timeZone + ").";
      rows = days.map((day, index) => [
        button(day + " " + dateLabel(cycle.cutoff + (index + 1) * dayMs) +
          " · " + totals[index], "day", index),
      ]);
    } else if (cycle.phase === "skip") {
      text = "🍕 No pizza meetup this week. Let’s vote for next week.";
    } else {
      const date = dateLabel(cycle.cutoff + (cycle.day + 1) * dayMs);
      text = "🍕 Pizza · " + days[cycle.day] + " " + date + " at " + cycle.time;
      if (cycle.phase === "done") text += " (finished)";
      const unavailable = Object.entries(cycle.votes)
        .filter(([, choices]) => choices.length > 0 && !choices.includes(cycle.day));
      const names = unavailable.map(([person]) => cycle.voterNames?.[person]).filter(Boolean);
      const visible = names.slice(0, 15);
      const extra = unavailable.length - visible.length;
      text += "\nDidn't select " + days[cycle.day] + " (" + unavailable.length + ") · " +
        (visible.join(", ") || "—");
      if (extra) text += " +" + extra;
      for (const [index, status] of statuses.entries()) {
        const names = Object.values(cycle.rsvps)
          .filter((rsvp) => rsvp.index === index).map((rsvp) => rsvp.name);
        const visible = names.slice(0, 15).join(", ");
        text += "\n" + status + " (" + names.length + ") · " + (visible || "—");
        if (names.length > 15) text += " +" + (names.length - 15);
      }
      if (cycle.phase === "lunch") {
        text += "\n\nTap to RSVP. The organizer can change the time below. A time change resets RSVPs.";
        rows = [
          statuses.map((status, index) => button(status, "rsvp", index)),
          times.filter((time) => time !== cycle.time)
            .map((time) => button(time, "time", times.indexOf(time))),
        ];
      }
    }
    return { text, reply_markup: { inline_keyboard: rows } };
  }

  async function render(cycle) {
    if (!cycle.dirty) return;
    if (cycle.message) {
      try {
        await call("editMessageText", {
          chat_id: cycle.destination.chat_id, message_id: cycle.message, ...card(cycle),
        });
      } catch (error) {
        if (!error.description?.includes("message is not modified")) throw error;
      }
    } else {
      const message = await call("sendMessage", { ...cycle.destination, ...card(cycle) });
      cycle.message = message.message_id;
    }
    cycle.dirty = false;
    await save();
  }

  async function open(key, destination, owner, now) {
    cycles[key] = {
      destination, owner, cutoff: cutoff(now), phase: "vote",
      votes: {}, voterNames: {}, rsvps: {}, time: "13:00", dirty: true,
    };
    await save();
    await render(cycles[key]);
  }

  return {
    async load() {
      try {
        cycles = JSON.parse(await readFile(file, "utf8"));
      } catch (error) {
        if (error.code !== "ENOENT") throw error;
      }
    },

    async start(destination, owner) {
      const key = String(destination.chat_id);
      if (cycles[key]) {
        return call("sendMessage", { ...destination, text: "🍕 Weekly pizza polls are already running in this group." });
      }
      await open(key, destination, owner, localTime());
    },

    async tick() {
      const now = localTime();
      for (const [key, cycle] of Object.entries(cycles)) {
        if (cycle.phase === "vote" && now >= cycle.cutoff) {
          const totals = counts(cycle);
          const high = Math.max(...totals);
          cycle.day = totals.indexOf(high);
          cycle.phase = high === 0 ? "skip" : "lunch";
          cycle.dirty = true;
          await save();
        }
        if (cycle.phase === "lunch" && now >= lunchEnd(cycle)) {
          cycle.phase = "done";
          cycle.dirty = true;
          await save();
        }
        await render(cycle);
        if (cycle.phase === "done" || cycle.phase === "skip") {
          await open(key, cycle.destination, cycle.owner, now);
        }
      }
    },

    async tap(query) {
      const [prefix, action, rawIndex] = (query.data ?? "").split(":");
      if (prefix !== "weekly") return;
      const answer = (text) => call("answerCallbackQuery", { callback_query_id: query.id, text });
      const cycle = cycles[String(query.message?.chat.id)];
      if (!cycle || cycle.message !== query.message.message_id) return answer("This card is closed.");
      const index = Number(rawIndex);
      const person = String(query.from.id);
      const now = localTime();

      let confirmation = "Saved";
      if (action === "day" && cycle.phase === "vote" && now < cycle.cutoff && days[index]) {
        const choices = cycle.votes[person] ?? [];
        cycle.votes[person] = choices.includes(index)
          ? choices.filter((choice) => choice !== index)
          : [...choices, index];
        cycle.voterNames ??= {};
        cycle.voterNames[person] = displayName(query.from);
        const selected = cycle.votes[person].map((choice) => days[choice]);
        confirmation = selected.length ? "Your days: " + selected.join(", ") : "Your votes are cleared.";
      } else if (cycle.phase === "lunch" && now < lunchEnd(cycle)) {
        if (action === "rsvp" && statuses[index]) {
          cycle.rsvps[person] = { index, name: displayName(query.from) };
        } else if (action === "time" && times[index]) {
          if (person !== cycle.owner) return answer("Ask the organizer to change the time.");
          if (times[index] === cycle.time) return answer("This time is already selected.");
          const next = { ...cycle, time: times[index] };
          if (now >= lunchEnd(next) - 3_600_000) return answer("That time has already passed.");
          cycle.time = next.time;
          cycle.rsvps = {};
        } else {
          return answer("That option is unavailable.");
        }
      } else {
        return answer("Voting has closed.");
      }

      cycle.dirty = true;
      await save();
      await answer(confirmation);
      await render(cycle);
    },
  };
}
