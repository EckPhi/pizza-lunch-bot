# 🍕 Pizza Bot for Telegram

Your group votes for a time, then RSVPs to the confirmed pizza meetup. Telegram shows
the names behind each vote when group members open the poll results.

## Set up once

1. In Telegram, open [@BotFather](https://t.me/BotFather), send `/newbot` and
   follow the prompts. Keep the token private.
2. Add your bot to the Telegram group as a regular member. Keep BotFather's
   default Group Privacy setting enabled. The bot needs permission to send
   messages and polls in the group.
3. With Node.js 22 or later installed, open Terminal in this folder. On macOS
   with the default zsh shell, run the following and paste the token at the
   hidden prompt.

   ```zsh
   read -rs 'TELEGRAM_BOT_TOKEN?Paste your BotFather token and press Enter: '
   export TELEGRAM_BOT_TOKEN
   node bot.mjs
   ```

   Leave the process running to handle new commands. For use while your
   computer is asleep, run it on an always-on machine with the token set as
   a secret environment variable. Run one copy per token.

## Start the weekly cycle

Send `/weekly@your_bot` in your group once, replacing `your_bot` with
your bot's username. The person who starts the cycle is the organizer.

- The bot posts a Tuesday–Friday pizza poll. Everyone can tap all
  days they can make. Tapping a selected day again removes that vote.
- Voting closes Monday at 20:00 in Europe/Vienna. The day with the most
  votes wins. Ties go to the earlier day. With no votes, the bot skips
  pizza that week and opens the next poll.
- The card becomes an RSVP card for the winning day at 13:00. Everyone
  can see which voters selected other days but did not select the winning
  day, plus the names under Going, Maybe and Can't make it. People who
  did not vote are not listed as unavailable. If you upgrade during an
  open poll, earlier votes still count, but their names are only known
  if they tap a day button again.
- The organizer can choose 12:00, 12:30, 13:00, 13:30 or 14:00 using
  the buttons. A time change clears old RSVPs so everyone can answer
  for the new time.
- An hour after the scheduled pizza meetup starts, the bot closes RSVPs and posts
  the following week's poll.

Keep the bot running for button taps and automatic transitions. Preserve
`lunch-state.json` in the working directory across restarts. The file
contains the current votes and attendee names. Set `LUNCH_STATE_FILE`
to a persistent location if needed, and set `LUNCH_TIME_ZONE` before
starting the cycle if your group uses a different time zone.

The original `/plan` and `/lunch` commands still work for one-off native
Telegram polls.

[Troubleshooting bot setup and group privacy](https://core.telegram.org/bots/features#privacy-mode)

To rename an existing bot in Telegram, send `/setname` to
[@BotFather](https://t.me/BotFather) and choose a display name such as
**🍕 Pizza Bot**. Your bot username and commands stay the same.
