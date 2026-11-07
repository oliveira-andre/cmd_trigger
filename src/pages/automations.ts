// Automations: when something happens (triggers), if the situation allows it (conditions), do
// these steps in order.

import { api, on, type AppInfo, type Automation, type Condition, type Step, type Trigger, type Weekday } from "../api";
import { button, dialog, field, h, toast, toggle } from "../dom";
import { icon, type IconName } from "../icons";
import { t } from "../i18n";
import { emojiPicker } from "./emoji";
import { openRunDialog } from "./commands";
import type { Cleanup } from "../main";

const DAYS: Weekday[] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
const DAY_LABEL: Record<Weekday, string> = { mon: "Mon", tue: "Tue", wed: "Wed", thu: "Thu", fri: "Fri", sat: "Sat", sun: "Sun" };

const TRIGGERS: { type: Trigger["type"]; label: string; icon: IconName; make: () => Trigger }[] = [
  { type: "schedule", label: "At a time", icon: "clock", make: () => ({ type: "schedule", at: "08:00", days: [] }) },
  { type: "interval", label: "Every few minutes", icon: "repeat", make: () => ({ type: "interval", minutes: 15 }) },
  { type: "startup", label: "When the app starts", icon: "sunrise", make: () => ({ type: "startup" }) },
  { type: "wake", label: "When the computer wakes", icon: "power", make: () => ({ type: "wake" }) },
  { type: "network", label: "Network change", icon: "wifi", make: () => ({ type: "network", network: "", event: "joined" }) },
  { type: "usb", label: "USB device", icon: "usb", make: () => ({ type: "usb", event: "connected", name: "" }) },
  { type: "display", label: "Display", icon: "monitor", make: () => ({ type: "display", event: "connected" }) },
  { type: "file", label: "File or folder changes", icon: "file", make: () => ({ type: "file", path: "", recursive: false }) },
  { type: "webhook", label: "Webhook", icon: "webhook", make: () => ({ type: "webhook", hook: "" }) },
];
const CONDITIONS: { type: Condition["type"]; label: string; make: () => Condition }[] = [
  { type: "network", label: "On a network", make: () => ({ type: "network", network: "", not: false }) },
  { type: "time", label: "Between times", make: () => ({ type: "time", after: "08:00", before: "22:00" }) },
  { type: "days", label: "On weekdays", make: () => ({ type: "days", days: ["mon", "tue", "wed", "thu", "fri"] }) },
];
const STEPS: { type: Step["type"]; label: string; icon: IconName; make: () => Step }[] = [
  { type: "command", label: "Run a command", icon: "terminal", make: () => ({ type: "command", command: "", params: "" }) },
  { type: "shell", label: "Shell line", icon: "braces", make: () => ({ type: "shell", line: "" }) },
  { type: "wait", label: "Wait", icon: "timer", make: () => ({ type: "wait", seconds: 5 }) },
  { type: "wakeOnLan", label: "Wake a computer", icon: "power", make: () => ({ type: "wakeOnLan", mac: "", broadcast: "" }) },
  { type: "notify", label: "Notification", icon: "bell", make: () => ({ type: "notify", title: "", body: "" }) },
];

function days(list: Weekday[]) {
  if (!list.length || list.length === 7) return t("every day");
  if (list.join() === "mon,tue,wed,thu,fri") return t("weekdays");
  if (list.join() === "sat,sun") return t("weekends");
  return list.map((d) => t(DAY_LABEL[d])).join(", ");
}

export function describeTrigger(tr: Trigger): string {
  switch (tr.type) {
    case "schedule": return t("At {time}, {days}", { time: tr.at, days: days(tr.days) });
    case "interval": return t("Every {n} min", { n: tr.minutes });
    case "startup": return t("When the app starts");
    case "wake": return t("When the computer wakes");
    case "network": return tr.network ? t(tr.event === "left" ? "Leaving {n}" : "Joining {n}", { n: tr.network.replace(/^router:/, "router ") }) : t("Any network change");
    case "usb": return `${t(tr.event === "disconnected" ? "USB unplugged" : "USB plugged in")}${tr.name ? `: ${tr.name}` : ""}`;
    case "display": return t(tr.event === "disconnected" ? "Display disconnected" : "Display connected");
    case "file": return t("{path} changes", { path: tr.path || "?" });
    case "webhook": return t("Webhook");
  }
}

function describeStep(s: Step): string {
  switch (s.type) {
    case "command": return `▶ ${s.command}${s.params ? ` ${s.params}` : ""}`;
    case "shell": return `$ ${s.line}`;
    case "wait": return t("wait {n} s", { n: s.seconds });
    case "wakeOnLan": return t("wake {mac}", { mac: s.mac });
    case "notify": return `🔔 ${s.title}`;
  }
}

export async function mountAutomations(root: HTMLElement, info: AppInfo, rest: string[]): Promise<Cleanup> {
  let automations: Automation[] = [];
  const list = h("ul", { class: "command-list" });
  const count = h("span", { class: "count" });
  root.append(
    h(
      "div",
      { class: "page" },
      h(
        "header",
        { class: "page-head" },
        h("div", { class: "page-title" }, h("h1", null, t("Automations")), count),
        h("div", { class: "head-fill" }),
        button(t("New automation"), { icon: "plus", kind: "primary", onClick: () => openEditor(null) }),
      ),
      h("div", { class: "scroll" }, list),
    ),
  );

  async function load() {
    try {
      automations = await api.loadAutomations();
    } catch (e) {
      list.replaceChildren(h("li", { class: "empty error" }, h("p", null, String(e))));
      return;
    }
    render();
  }

  async function save(next: Automation[]) {
    try {
      await api.saveAutomations(next);
      automations = next;
      render();
      return true;
    } catch (e) {
      toast(String(e), "error");
      return false;
    }
  }

  function render() {
    count.textContent = t("{n} automations", { n: automations.length });
    if (!automations.length) {
      list.replaceChildren(
        h(
          "li",
          { class: "empty" },
          icon("zap", 28),
          h("p", { class: "empty-title" }, t("No automations yet.")),
          h("p", null, t("Run commands on a schedule, when you get home, when a device is plugged in, or from a webhook.")),
        ),
      );
      return;
    }
    list.replaceChildren(
      ...automations.map((a, index) => {
        const enabled = toggle(a.enabled, "", async (onNow) => {
          const next = [...automations];
          next[index] = { ...a, enabled: onNow };
          await save(next);
        });
        return h(
          "li",
          { class: `command${a.enabled ? "" : " disabled"}` },
          h("div", { class: "command-icon" }, a.icon ? h("span", { class: "emoji" }, a.icon) : icon("zap", 20)),
          h(
            "div",
            { class: "command-body" },
            h("div", { class: "command-name" }, a.name),
            h(
              "div",
              { class: "chips" },
              a.triggers.length ? a.triggers.map((tr) => h("span", { class: "chip chip-fg" }, describeTrigger(tr))) : h("span", { class: "chip" }, t("Run by hand")),
              a.conditions.length ? h("span", { class: "chip" }, a.conditions.length === 1 ? t("1 condition") : t("{n} conditions", { n: a.conditions.length })) : null,
              a.ground === "background" ? h("span", { class: "chip chip-bg" }, t("background service")) : null,
            ),
            h("code", { class: "command-line" }, a.steps.map(describeStep).join("  →  ")),
          ),
          h(
            "div",
            { class: "command-actions" },
            enabled.root,
            button(null, { icon: "play", title: t("Run now"), small: true, kind: "primary", onClick: () => openRunDialog("automation", a.id, { name: a.name, allowParams: a.allowParams, hasOff: false }) }),
            button(null, { icon: "edit", title: t("Edit"), small: true, onClick: () => openEditor(index) }),
            button(null, {
              icon: "trash",
              title: t("Delete"),
              small: true,
              kind: "danger",
              onClick: () => {
                const body = h("div", { class: "dialog-body" }, h("p", null, t("It stops running and disappears from HomeKit, Home Assistant and the server.")));
                const { close } = dialog(t("Delete {name}?", { name: a.name }), body);
                body.append(h("div", { class: "dialog-actions" }, button(t("Cancel"), { onClick: close }), button(t("Delete"), { kind: "danger", icon: "trash", onClick: async () => { if (await save(automations.filter((_, i) => i !== index))) close(); } })));
              },
            }),
          ),
        );
      }),
    );
  }

  async function openEditor(index: number | null) {
    const commandNames = (await api.loadCommands()).map((c) => String(c.name));
    const apiInfo = await api.apiGet().catch(() => null);
    const original: Automation = index === null
      ? { id: await api.newId(10), name: "", enabled: true, icon: "", description: "", ground: "foreground", triggers: [], conditions: [], steps: [], stopOnError: true, allowParams: false }
      : structuredClone(automations[index]);
    const a = structuredClone(original);

    const name = h("input", { class: "input", value: a.name, placeholder: t("e.g. Good morning") });
    const description = h("input", { class: "input", value: a.description, placeholder: t("What it does (optional, shown to AI assistants)") });
    const enabled = toggle(a.enabled, t("On"));
    const allowParams = toggle(a.allowParams, t("Takes parameters ({params} in steps)"));
    const stopOnError = toggle(a.stopOnError, t("Stop when a step fails"));
    const ground = h("select", { class: "input select", disabled: !info.backgroundSupported }, h("option", { value: "foreground" }, t("This app")), info.backgroundSupported ? h("option", { value: "background" }, t("Background service")) : null);
    ground.value = a.ground;
    let chosenIcon = a.icon;
    const picker = emojiPicker(chosenIcon, (e) => (chosenIcon = e), "zap");

    const triggersBox = h("div", { class: "rows" });
    const conditionsBox = h("div", { class: "rows" });
    const stepsBox = h("div", { class: "rows" });
    const usbNames = h("datalist", { id: "usb-names" });
    api.usbDevices().then((names) => usbNames.replaceChildren(...names.map((n) => h("option", { value: n })))).catch(() => {});

    const text = (value: string, set: (v: string) => void, placeholder = "", mono = false) =>
      h("input", { class: `input${mono ? " mono" : ""}`, value, placeholder, spellcheck: "false", onInput: (e: Event) => set((e.target as HTMLInputElement).value) });
    const time = (value: string, set: (v: string) => void) => h("input", { class: "input time", type: "time", value, onInput: (e: Event) => set((e.target as HTMLInputElement).value) });
    const number = (value: number, set: (v: number) => void, min = 0, step = "1") =>
      h("input", { class: "input number", type: "number", min, step, value: String(value), onInput: (e: Event) => set(Number((e.target as HTMLInputElement).value)) });
    const select = <T extends string>(value: T, options: [T, string][], set: (v: T) => void) => {
      const s = h("select", { class: "input select", onChange: () => set(s.value as T) }, ...options.map(([v, l]) => h("option", { value: v }, t(l))));
      s.value = value;
      return s;
    };
    const dayPicker = (list: Weekday[], set: (d: Weekday[]) => void) => {
      const box = h("div", { class: "days" });
      const draw = () =>
        box.replaceChildren(
          ...DAYS.map((d) =>
            h("button", { type: "button", class: `day${list.includes(d) ? " active" : ""}`, onClick: () => { list = list.includes(d) ? list.filter((x) => x !== d) : DAYS.filter((x) => x === d || list.includes(x)); set(list); draw(); } }, t(DAY_LABEL[d]).slice(0, 2)),
          ),
        );
      draw();
      return box;
    };
    const networkField = (value: string, set: (v: string) => void) => {
      const input = text(value, set, t("Wi-Fi name, or router:aa:bb:cc:dd:ee:ff"), true);
      return h(
        "div",
        { class: "inline" },
        input,
        button(t("This network"), {
          small: true,
          icon: "wifi",
          onClick: async () => {
            const n = await api.currentNetwork();
            if (!n) return toast(t("Not connected to a network."), "error");
            input.value = n.id;
            set(n.id);
            toast(n.label);
          },
        }),
      );
    };
    const hookUrl = (hook: string) => `http://${apiInfo?.config.allowLan ? "<this-computer>" : "127.0.0.1"}:${(apiInfo?.config.port ?? 7171) + (ground.value === "background" ? 1 : 0)}/hooks/${hook}`;

    const row = (label: string, ic: IconName | null, body: HTMLElement[], remove: () => void, move?: [() => void, () => void]) =>
      h(
        "div",
        { class: "row-card" },
        h("div", { class: "row-head" }, ic ? icon(ic, 14) : null, h("strong", null, t(label)), h("span", { class: "head-fill" }),
          move ? button(null, { icon: "up", small: true, kind: "ghost", title: t("Move up"), onClick: move[0] }) : null,
          button(null, { icon: "close", small: true, kind: "ghost", title: t("Remove"), onClick: remove })),
        h("div", { class: "row-body" }, ...body),
      );

    function drawTriggers() {
      triggersBox.replaceChildren(
        ...a.triggers.map((tr, i) => {
          const def = TRIGGERS.find((d) => d.type === tr.type)!;
          const remove = () => { a.triggers.splice(i, 1); drawTriggers(); };
          let body: HTMLElement[] = [];
          if (tr.type === "schedule") body = [time(tr.at, (v) => (tr.at = v)), dayPicker(tr.days, (d) => (tr.days = d))];
          if (tr.type === "interval") body = [number(tr.minutes, (v) => (tr.minutes = v), 1), h("span", { class: "muted" }, t("minutes"))];
          if (tr.type === "network") body = [select(tr.event, [["joined", "Joining"], ["left", "Leaving"]], (v) => (tr.event = v)), networkField(tr.network, (v) => (tr.network = v)), h("span", { class: "muted small" }, t("Empty: any change."))];
          if (tr.type === "usb") {
            const nameInput = text(tr.name, (v) => (tr.name = v), t("Name contains (empty = any device)"));
            nameInput.setAttribute("list", "usb-names");
            body = [select(tr.event, [["connected", "Plugged in"], ["disconnected", "Unplugged"]], (v) => (tr.event = v)), nameInput];
          }
          if (tr.type === "display") body = [select(tr.event, [["connected", "Connected"], ["disconnected", "Disconnected"]], (v) => (tr.event = v))];
          if (tr.type === "file") body = [text(tr.path, (v) => (tr.path = v), "~/Downloads", true), toggle(tr.recursive, t("Include subfolders"), (v) => (tr.recursive = v)).root];
          if (tr.type === "webhook") {
            const url = h("code", { class: "hook-url" }, hookUrl(tr.hook));
            body = [url, button(t("Copy"), { small: true, icon: "copy", onClick: () => navigator.clipboard.writeText(hookUrl(tr.hook)).then(() => toast(t("Copied."), "success")) }),
              h("span", { class: "muted small" }, t("POST or GET. Send parameters as ?params=, a JSON {\"params\": …} body, or plain text."))];
          }
          if (tr.type === "startup") body = [h("span", { class: "muted small" }, t("At login for the app; at boot for the background service."))];
          return row(def.label, def.icon, body, remove);
        }),
      );
    }

    function drawConditions() {
      conditionsBox.replaceChildren(
        ...a.conditions.map((c, i) => {
          const def = CONDITIONS.find((d) => d.type === c.type)!;
          const remove = () => { a.conditions.splice(i, 1); drawConditions(); };
          let body: HTMLElement[] = [];
          if (c.type === "network") body = [select(c.not ? "not" : "on", [["on", "Only on"], ["not", "Not on"]], (v) => (c.not = v === "not")), networkField(c.network, (v) => (c.network = v))];
          if (c.type === "time") body = [h("span", { class: "muted" }, t("from")), time(c.after, (v) => (c.after = v)), h("span", { class: "muted" }, t("to")), time(c.before, (v) => (c.before = v))];
          if (c.type === "days") body = [dayPicker(c.days, (d) => (c.days = d))];
          return row(def.label, null, body, remove);
        }),
      );
    }

    function drawSteps() {
      stepsBox.replaceChildren(
        ...a.steps.map((s, i) => {
          const def = STEPS.find((d) => d.type === s.type)!;
          const remove = () => { a.steps.splice(i, 1); drawSteps(); };
          const up = () => { if (i > 0) { [a.steps[i - 1], a.steps[i]] = [a.steps[i], a.steps[i - 1]]; drawSteps(); } };
          let body: HTMLElement[] = [];
          if (s.type === "command") {
            const pick = h("select", { class: "input select", onChange: () => (s.command = pick.value) }, h("option", { value: "" }, t("Choose a command…")), ...commandNames.map((n) => h("option", { value: n }, n)));
            pick.value = s.command;
            body = [pick, text(s.params, (v) => (s.params = v), t("Parameters (optional, {params} = the automation's)"), true)];
          }
          if (s.type === "shell") body = [text(s.line, (v) => (s.line = v), "say \"Good morning\"", true)];
          if (s.type === "wait") body = [number(s.seconds, (v) => (s.seconds = v), 0, "0.5"), h("span", { class: "muted" }, t("seconds"))];
          if (s.type === "wakeOnLan") body = [text(s.mac, (v) => (s.mac = v), "aa:bb:cc:dd:ee:ff", true), text(s.broadcast, (v) => (s.broadcast = v), t("Broadcast address (optional)"), true)];
          if (s.type === "notify") body = [text(s.title, (v) => (s.title = v), t("Title")), text(s.body, (v) => (s.body = v), t("Message (optional)"))];
          return row(`${i + 1}. ${t(def.label)}`, def.icon, body, remove, [up, () => {}]);
        }),
      );
    }

    const addMenu = <D extends { label: string; make: () => unknown; icon?: IconName }>(defs: D[], add: (item: ReturnType<D["make"]>) => void | Promise<void>) =>
      h("div", { class: "add-menu" }, ...defs.map((d) => button(t(d.label), { small: true, icon: d.icon ?? "plus", onClick: async () => { await add(d.make() as ReturnType<D["make"]>); } })));

    drawTriggers();
    drawConditions();
    drawSteps();

    const form = h(
      "form",
      { class: "dialog-body form-grid", onSubmit: (e: Event) => e.preventDefault() },
      h("div", { class: "row-2" }, field(t("Name"), name), field(t("Runs in"), ground)),
      field(t("Description"), description),
      h("div", { class: "toggles" }, enabled.root, stopOnError.root, allowParams.root),
      h("h3", { class: "section-title" }, icon("zap", 16), t("When")),
      h("p", { class: "muted small" }, t("Any of these starts it. With none, it runs only when you (or HomeKit, Home Assistant, the server, an AI) run it.")),
      triggersBox,
      addMenu(TRIGGERS, async (tr) => {
        if (tr.type === "webhook") tr.hook = await api.newId(20);
        a.triggers.push(tr);
        drawTriggers();
      }),
      h("h3", { class: "section-title" }, icon("check", 16), t("Only if")),
      conditionsBox,
      addMenu(CONDITIONS, (c) => { a.conditions.push(c); drawConditions(); }),
      h("h3", { class: "section-title" }, icon("play", 16), t("Do")),
      stepsBox,
      addMenu(STEPS, (s) => { a.steps.push(s); drawSteps(); }),
      field(t("Icon"), picker),
      usbNames,
    );
    const { close } = dialog(index === null ? t("New automation") : t("Edit automation"), form, { wide: true });
    form.append(
      h(
        "div",
        { class: "dialog-actions" },
        button(t("Cancel"), { onClick: close }),
        button(t("Save"), {
          kind: "primary",
          icon: "save",
          type: "submit",
          onClick: async () => {
            a.name = name.value.trim();
            a.description = description.value.trim();
            a.enabled = enabled.input.checked;
            a.allowParams = allowParams.input.checked;
            a.stopOnError = stopOnError.input.checked;
            a.ground = ground.value as Automation["ground"];
            a.icon = chosenIcon;
            const next = [...automations];
            if (index === null) next.push(a);
            else next[index] = a;
            if (await save(next)) close();
          },
        }),
      ),
    );
    name.focus();
  }

  await load();
  if (rest[0]) {
    const index = automations.findIndex((a) => a.id === rest[0]);
    if (index >= 0) openEditor(index);
  }
  const unlisten = await on("catalog-changed", () => load());
  return () => unlisten();
}
