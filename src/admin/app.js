const login = document.querySelector("#login");
const input = document.querySelector("#token");
const actions = document.querySelector("#actions");
const refresh = document.querySelector("#refresh");
const lock = document.querySelector("#lock");
const directory = document.querySelector("#directory");
const status = document.querySelector("#status");
let token = "";
let snapshot;

function element(tag, text = "") {
  const node = document.createElement(tag);
  node.textContent = text;
  return node;
}

function signOut() {
  token = "";
  snapshot = undefined;
  input.value = "";
  directory.replaceChildren();
  directory.hidden = true;
  actions.hidden = true;
  login.hidden = false;
  input.focus();
}

function render(data) {
  directory.replaceChildren();
  for (const [kind, entries, allowlist] of [
    ["group", data.groups, data.groups_allowlist],
    ["user", data.users, data.users_allowlist],
  ]) {
    const section = element("fieldset");
    const title = kind === "group" ? "Groups" : "Users";
    section.setAttribute("aria-label", title);
    const label = element("label");
    const toggle = element("input");
    toggle.type = "checkbox";
    toggle.checked = allowlist;
    toggle.addEventListener("change", () => load({
      action: "set_allowlist", kind, enabled: toggle.checked,
    }));
    label.append(toggle, " Only allow entries marked Allow");
    section.append(element("h2", title), label);

    const records = Object.entries(entries);
    if (!records.length) {
      section.append(element("p", "No activity recorded yet."));
      directory.append(section);
      continue;
    }
    const table = element("table");
    const head = table.createTHead().insertRow();
    const columns = kind === "user" ? ["Name and ID", "Groups seen in", "Access"] : ["Name and ID", "Access"];
    for (const column of columns) head.append(element("th", column));
    const body = table.createTBody();
    for (const [id, entry] of records) {
      const row = body.insertRow();
      row.insertCell().append(element("strong", entry.name || "Unknown name"), element("small", id));
      if (kind === "user") {
        const names = entry.groups.map((id) => data.groups[id]?.name || id);
        row.insertCell().textContent = names.join(", ") || "—";
      }
      const select = element("select");
      select.setAttribute("aria-label", "Access for " + (entry.name || id));
      for (const [value, text] of [
        ["default", allowlist ? "Default (blocked)" : "Default (allowed)"],
        ["allow", "Allow"],
        ["block", "Block"],
      ]) {
        const option = element("option", text);
        option.value = value;
        select.append(option);
      }
      select.value = entry.rule;
      select.addEventListener("change", () => load({action: "set_rule", kind, id, rule: select.value}));
      row.insertCell().append(select);
    }
    const wrap = element("div");
    wrap.className = "table-wrap";
    wrap.append(table);
    section.append(wrap);
    directory.append(section);
  }
  directory.hidden = false;
  actions.hidden = false;
  login.hidden = true;
}

async function load(change) {
  login.querySelector("button").disabled = true;
  directory.querySelectorAll("fieldset").forEach((section) => { section.disabled = true; });
  lock.disabled = true;
  refresh.disabled = true;
  status.textContent = change ? "Saving…" : "Loading…";
  try {
    const response = await fetch("/api", {
      method: change ? "POST" : "GET",
      headers: {Authorization: "Bearer " + token, "Content-Type": "application/json"},
      body: change ? JSON.stringify(change) : undefined,
    });
    const data = await response.json();
    if (response.status === 401) signOut();
    if (!response.ok) throw new Error(data.error || "Could not load access settings.");
    snapshot = data;
    render(snapshot);
    status.textContent = change ? "Saved." : "";
  } catch (error) {
    if (snapshot) render(snapshot);
    status.textContent = error.message || "Could not load access settings.";
  } finally {
    login.querySelector("button").disabled = false;
    lock.disabled = false;
    refresh.disabled = false;
  }
}

login.addEventListener("submit", (event) => {
  event.preventDefault();
  token = input.value;
  input.value = "";
  load();
});
lock.addEventListener("click", () => {
  signOut();
  status.textContent = "";
});
refresh.addEventListener("click", () => load());
