const telegram = window.Telegram?.WebApp;
const heading = document.querySelector("#heading");
const subtitle = document.querySelector("#subtitle");
const panel = document.querySelector("#panel");
const status = document.querySelector("#status");
let pending = false;

function element(tag, text, className) {
  const node = document.createElement(tag);
  if (text) node.textContent = text;
  if (className) node.className = className;
  return node;
}

function choice(label, selected, view, action, index, detail) {
  const button = element("button", "", detail ? "option" : "");
  button.type = "button";
  button.setAttribute("aria-pressed", String(selected));
  button.append(element("strong", label));
  if (detail) button.append(element("span", detail, "meta"));
  button.disabled = view.message == null;
  button.addEventListener("click", () => load({ message: view.message, action, index }));
  return button;
}

function render(view) {
  panel.replaceChildren();
  if (view.lunch) {
    const lunch = view.lunch;
    heading.textContent = lunch.day + " pizza";
    subtitle.textContent = lunch.date + " · " + lunch.time;
    if (view.rsvp) {
      const section = element("section", "", "box");
      const choices = element("div", "", "choices");
      lunch.responses.forEach(({label}, index) => choices.append(choice(label, lunch.selected === index, view, "rsvp", index)));
      section.append(element("h2", "Can you make it?"), choices);
      panel.append(section);
    }
    const guests = element("section", "", "box");
    guests.append(element("h2", "At the table"));
    for (const group of lunch.responses) {
      guests.append(element("p", group.label + " · " + group.count, "tag"));
      const extra = group.count - group.names.length;
      const names = group.names.join(", ") || "—";
      guests.append(element("p", names + (extra > 0 ? " +" + extra : ""), "names"));
    }
    if (lunch.unavailable.length) {
      guests.append(element("p", "Didn’t select " + lunch.day, "tag"));
      guests.append(element("p", lunch.unavailable.join(", "), "names"));
    }
    panel.append(guests);
    if (lunch.times.length) {
      const section = element("section", "", "box");
      const choices = element("div", "", "choices");
      lunch.times.forEach((time, index) => {
        if (time !== lunch.time) choices.append(choice(time, false, view, "time", index));
      });
      section.append(element("h2", "Change the time"), choices,
        element("p", "A new time clears existing RSVPs.", "meta"));
      panel.append(section);
    }
  } else {
    heading.textContent = "Pizza this week?";
    subtitle.textContent = view.voting ? "Tap every day that works for you." : "Voting is closed. Check back for the plan.";
    panel.append(element("p", "Voting closes " + view.deadline, "tag"));
    const grid = element("div", "", "grid");
    view.options.forEach((option, index) => {
      const detail = option.date + " · " + option.count + (option.count === 1 ? " vote" : " votes");
      const button = choice(option.label, option.selected, view, "day", index, detail);
      button.disabled ||= !view.voting;
      grid.append(button);
    });
    panel.append(grid);
  }
}

async function load(selection = null) {
  if (pending || !telegram?.initData) return;
  pending = true;
  panel.setAttribute("aria-busy", "true");
  try {
    const response = await fetch("/api/plan", {
      method: "POST",
      headers: { Authorization: "tma " + telegram.initData, "Content-Type": "application/json" },
      body: JSON.stringify(selection),
    });
    const view = await response.json();
    if (!response.ok) throw new Error(view.error || "Could not load the pizza plan.");
    const focused = document.activeElement;
    const index = Array.from(panel.querySelectorAll("button")).indexOf(focused);
    render(view);
    if (index >= 0) panel.querySelectorAll("button")[index]?.focus();
    status.textContent = view.notice || "";
  } catch (error) {
    status.textContent = error.message || "Could not load the pizza plan.";
  } finally {
    pending = false;
    panel.setAttribute("aria-busy", "false");
  }
}

telegram?.ready();
if (telegram?.initData) {
  load();
  telegram.onEvent("activated", () => load());
} else {
  panel.setAttribute("aria-busy", "false");
  status.textContent = "Open Pizza Lunch from the button in your Telegram group.";
}
