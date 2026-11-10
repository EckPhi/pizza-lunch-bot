import { weeklyLunches } from "./weekly.mjs";
import { pathToFileURL } from "node:url";
import { setTimeout as delay } from "node:timers/promises";

const help = [
  "🍕 Pizza Bot",
  "",
  "Use these commands in the group. Add @BOT_USERNAME after the command name to address me directly.",
  "",
  "/weekly starts a recurring pizza poll in this group.",
  "",
  "/plan Tuesday 12:00 | Wednesday 12:30 | Friday 12:00",
  "Vote for every time you can make.",
  "",
  "/lunch Wednesday 12:30 at your favorite pizzeria",
  "Confirm the time and place and see who's coming.",
].join("\n");

const commands = [
  { command: "weekly", description: "Start the weekly pizza poll" },
  { command: "plan", description: "Poll the group for available pizza times" },
  { command: "lunch", description: "Post pizza meetup details and collect RSVPs" },
  { command: "help", description: "Show pizza bot examples" },
];

export async function handleUpdate(update, username, call, weekly) {
  const message = update.message;
  if (!message?.text || message.entities?.[0]?.type !== "bot_command") return;

  const [address] = message.text.split(/\s/, 1);
  const [command, recipient] = address.toLowerCase().split("@");
  if (recipient && recipient !== username.toLowerCase()) return;
  if (!["/start", "/help", "/plan", "/lunch", "/weekly"].includes(command)) return;

  const destination = { chat_id: message.chat.id };
  if (message.is_topic_message) {
    destination.message_thread_id = message.message_thread_id;
  }
  const reply = (text) => call("sendMessage", { ...destination, text });

  if (command === "/start" || command === "/help") {
    return reply(help.replace("@BOT_USERNAME", "@" + username));
  }
  if (!["group", "supergroup"].includes(message.chat.type)) {
    return reply("Add me to your pizza group and use this command there.");
  }

  if (command === "/weekly") {
    if (!message.from || message.sender_chat) {
      return reply("Send this command as yourself to become the organizer.");
    }
    return weekly.start(destination, String(message.from.id));
  }

  const details = message.text.slice(address.length).trim();
  const planning = command === "/plan";
  const question = planning
    ? "🍕 When can you join for pizza? Select every time that works."
    : "🍕 Pizza · " + details;
  const choices = planning
    ? details.split("|").map((slot) => slot.trim())
    : ["Going", "Maybe", "Can't make it"];

  if (planning && (choices.length < 2 || choices.length > 12 ||
      new Set(choices).size !== choices.length ||
      choices.some((choice) => !choice || choice.length > 100))) {
    return reply("Give me 2–12 different times separated by |, up to 100 characters each.");
  }
  if (!planning && (!details || question.length > 300)) {
    return reply("Add a short date, time and place. Example: /lunch Wednesday 12:30 at your favorite pizzeria");
  }

  return call("sendPoll", {
    ...destination,
    question,
    options: choices.map((text) => ({ text })),
    is_anonymous: false,
    allows_multiple_answers: planning,
  });
}

async function run() {
  const token = process.env.TELEGRAM_BOT_TOKEN?.trim();
  if (!token || !/^\d+:[\w-]+$/.test(token)) {
    throw new Error("Set TELEGRAM_BOT_TOKEN to the token from BotFather.");
  }

  async function call(method, body = {}) {
    let result;
    try {
      const response = await fetch("https://api.telegram.org/bot" + token + "/" + method, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
        signal: AbortSignal.timeout(45_000),
      });
      result = await response.json();
    } catch {
      throw new Error(method + " could not reach Telegram or read its response.");
    }

    if (!result.ok) {
      const error = new Error(method + " failed (Telegram error " + result.error_code + ").");
      error.description = result.description;
      error.code = result.error_code;
      error.retryAfter = result.parameters?.retry_after;
      throw error;
    }
    return result.result;
  }

  const { username } = await call("getMe");
  await call("setMyCommands", { commands });
  console.log("Listening as @" + username + ". Press Ctrl-C to stop.");

  const weekly = weeklyLunches(call, process.env.LUNCH_STATE_FILE || "lunch-state.json",
    process.env.LUNCH_TIME_ZONE || "Europe/Vienna");
  await weekly.load();

  let offset;
  while (true) {
    try {
      await weekly.tick();
    } catch (error) {
      console.error("Weekly update failed:", error.message);
    }
    let updates;
    try {
      updates = await call("getUpdates", {
        offset,
        timeout: 25,
        allowed_updates: ["message", "callback_query"],
      });
    } catch (error) {
      if (error.code === 401 || error.code === 409) {
        throw new Error(error.message + " Check your token, remove any webhook and run only one copy.");
      }
      console.error(error.message);
      await delay((error.retryAfter ?? 5) * 1000);
      continue;
    }

    for (const update of updates) {
      try {
        if (update.callback_query) {
          await weekly.tap(update.callback_query);
        } else {
          await handleUpdate(update, username, call, weekly);
        }
      } catch (error) {
        console.error(error.message);
        // A lost response can mean Telegram already posted the poll.
        // Avoid retrying the send automatically and creating a duplicate.
      }
      offset = update.update_id + 1;
    }
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  run().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
