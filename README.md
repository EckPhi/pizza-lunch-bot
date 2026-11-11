# 🍕 Pizza Bot for Telegram

Your group votes for a time, then RSVPs to the confirmed pizza meetup. Telegram shows
the names behind each vote when group members open the poll results.

## Set up once

1. In Telegram, open [@BotFather](https://t.me/BotFather), send `/newbot` and
   follow the prompts. Keep the token private.
2. Add your bot to the Telegram group as a regular member. Keep BotFather's
   default Group Privacy setting enabled. The bot needs permission to send
   messages and polls in the group.
3. With a current stable Rust toolchain installed, open Terminal in this folder. On macOS
   with the default zsh shell, run the following and paste the token at the
   hidden prompt.

   ```zsh
   read -rs 'TELEGRAM_BOT_TOKEN?Paste your BotFather token and press Enter: '
   export TELEGRAM_BOT_TOKEN
   cargo run --locked --release
   ```

   Leave the process running to handle new commands. For use while your
   computer is asleep, run it on an always-on machine with the token set as
   a secret environment variable. Run one copy per token.

### Upgrading from JavaScript

Stop the old bot before starting the Rust version. Keep the same Telegram token,
working directory, time zone and state file. Rust reads the existing state and
uses the same buttons and message IDs, so an open poll can continue in place.
To roll back, stop Rust and start the previous JavaScript version with the same
settings. Run only one version at a time.

## Find the bot commands in Telegram

Once the bot has started, type `/` in the group message field to see
Telegram's command suggestions. Select the entry for your Pizza Lunch Bot.

- `/weekly` starts the recurring pizza poll.
- `/status` shows the leading day and its vote count, or the scheduled weekly lunch.
- `/plan` creates a one-off availability poll. Add times separated by `|`.
- `/lunch` creates a one-off RSVP poll. Add the date, time, and place.
- With the Mini App configured, send `/plan` or `/lunch` on its own to
  get a form. Submitting posts the poll in the group or topic where
  the bot sent the form link. You can use the forms without starting `/weekly`.
- `/help` shows examples. In a group you can also send `/help@your_bot`
  using the bot's actual username.

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

## Docker

Build for your machine from the repository root.

```sh
docker build -t pizza-lunch-bot .
```

With `TELEGRAM_BOT_TOKEN` already exported in your shell, start one container
and keep its state in a named volume.

```sh
docker run -d --name pizza-lunch-bot --restart unless-stopped \
  --env TELEGRAM_BOT_TOKEN --env LUNCH_TIME_ZONE=Europe/Vienna \
  --volume pizza-lunch-state:/data pizza-lunch-bot
```

The container runs as UID 10001 and stores the poll in
`/data/lunch-state.json`. If you migrate an existing file into the volume,
ensure UID 10001 can read the file and write to the directory. Keep the volume
when replacing the container.

## GitHub Actions

Pull requests and pushes to `main` run formatting, Clippy, tests, and a
container smoke test. A successful push to `main` publishes the tested Linux
amd64 image to GitHub Container Registry as `latest` and with the commit SHA
as a tag. The package may initially be private. Its GitHub package settings
control whether anonymous pulls are allowed.

GitHub hosts the code and image. Run the container on an always-on Docker host
to receive Telegram updates and keep the weekly schedule.

Each CI run generates a Markdown guide by running the bot with sample users,
a fixed clock, and a mock Telegram API. Successful builds on `main` publish
the [message-flow guide](https://eckphi.github.io/pizza-lunch-bot/) to GitHub
Pages. Download the Markdown as **pizza-message-flows** from the workflow run
summary or its Artifacts section. To generate it locally, run the following
command.

```sh
PIZZA_FLOW_DOCS_FILE=target/message-flows.md cargo test --locked generate_message_flow_guide
```

## Releases

To release, push or merge your changes to `main`, then tag the commit with
the version you want to publish.

```sh
git fetch origin main
git tag -a v0.2.0 origin/main -m 'Release v0.2.0'
git push origin v0.2.0
```

CI checks that the commit is on `main`, then sets the Cargo package version
in its build checkout from the tag. It publishes the tested image under the
same version, such as `ghcr.io/eckphi/pizza-lunch-bot:0.2.0`.
Once that succeeds, the workflow creates a GitHub Release with generated
notes and the image reference. Use the version tag in deployments and
create a new version for subsequent changes.

## Telegram Mini App

The optional Pizza Lunch Mini App shows selectable day tiles, the confirmed
lunch, and the RSVP list. The organizer gets time controls on the same screen.
The Mini App and the Telegram poll use the same saved state.

1. Run the bot with `PIZZA_MINI_APP_SHORT_NAME` set to the Mini App short
   name you plan to register (for example, `pizza`). The Rust process serves
   the app at port 8080 by default. For Docker, add
   `--env PIZZA_MINI_APP_SHORT_NAME=pizza --publish 127.0.0.1:8080:8080`
   to the run command. For a local process behind a proxy, you can set
   `PIZZA_MINI_APP_LISTEN=127.0.0.1:8080`.
2. Put an HTTPS reverse proxy in front of that port and serve the app at the
   root of its domain. In BotFather, use `/newapp` to register that HTTPS
   URL and the matching short name for your bot.
3. Make the bot an administrator in the Telegram group so Telegram can verify
   each Mini App user's membership. Start the weekly cycle with `/weekly`.
   Tap **🍕 Open Pizza Lunch** on the card, or send `/pizza@your_bot`.

For Runtipi, an HTTPS route to container port 8080 and the Mini App short
name environment variable must be configured. The original headless store
entry uses Telegram buttons only. The Mini App listener runs when the short
name is configured.

## Admin website

Set `PIZZA_ADMIN_TOKEN` to a separate random secret of at least 32 visible
ASCII characters, for example one generated with `openssl rand -hex 32`.
Restart the bot and open `http://127.0.0.1:8081/` on the host. Sign in
with the token. Set `PIZZA_ADMIN_LISTEN` to override the admin bind address.
For remote access, route an HTTPS reverse proxy to the admin listener on
port 8081 and open the root of that HTTPS domain. The admin website can run
on its own without a BotFather Mini App.

For Docker with a reverse proxy on the host, add
`--env PIZZA_ADMIN_TOKEN --env PIZZA_ADMIN_LISTEN=0.0.0.0:8081`
and `--publish 127.0.0.1:8081:8081` to the run command. Set
`PIZZA_ADMIN_TOKEN` in the host environment before running it.

The directory lists Telegram group and user IDs, names observed by the bot,
and which groups each user interacted in. An existing weekly state file
supplies historical IDs. New entries appear when someone uses a bot command,
weekly button, or the Mini App, including attempts denied by an allowlist.

- **Allow** and **Block** override the default for that ID. **Default** follows
  the corresponding group or user setting.
- **Only allow entries marked Allow** enables an allowlist for groups or
  users. Both rules must permit a group interaction. Mark your own test group
  and user as **Allow** before enabling both modes.
- Blocking a group pauses scheduled weekly messages. Blocking a user prevents
  new bot interactions by that account. Telegram handles votes on native
  one-off polls, so those polls follow Telegram's group permissions.
- When access management is active, use commands from your Telegram account.
  Telegram hides the account behind messages sent as an anonymous admin or channel.

The directory and its rules are saved alongside the weekly state in
`lunch-state.access.json` by default. Keep both files on the persistent
volume. Removing the admin token disables the website while preserving the
saved access rules.
