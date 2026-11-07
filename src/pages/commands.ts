// Commands: what this computer can run. Add, edit, delete, run, start from an example, or
// edit commands.json directly. Saving writes the file; the agent picks it up everywhere
// (HomeKit, Home Assistant, the server, MCP) on its own.

import { api, flag, on, type AppInfo, type Command, type Kind, type RunRecord } from "../api";
import { button, dialog, field, h, toast, toggle } from "../dom";
import { icon } from "../icons";
import { t } from "../i18n";
import { appPicker } from "./apps";
import { emojiPicker } from "./emoji";
import type { Cleanup } from "../main";

type SortKey = "name" | "command" | "ground";

const PARAMS_VARIABLE = "$CMDTRIGGER_PARAMS";

/// Show a finished run: a toast, and the output when there is some.
export function showRun(record: RunRecord) {
  const ok = record.status === "succeeded";
  toast(`${record.name}: ${ok ? t("done") : t("failed")}${record.exitCode && !ok ? ` (exit ${record.exitCode})` : ""}`, ok ? "success" : "error");
  if (record.output.trim()) {
    const body = h("div", { class: "dialog-body" }, h("pre", { class: "output" }, record.output));
    const { close } = dialog(record.name, body, { wide: true });
    body.append(h("div", { class: "dialog-actions" }, button(t("Close"), { onClick: close })));
  }
}

/// Run something, asking for parameters first when it takes them.
export async function openRunDialog(kind: Kind, target: string, opts: { name?: string; allowParams?: boolean; hasOff?: boolean } = {}) {
  let { name, allowParams, hasOff } = opts;
  if (name === undefined) {
    if (kind === "command") {
      const c = (await api.loadCommands()).find((c) => String(c.name).toLowerCase() === target.toLowerCase());
      name = String(c?.name ?? target);
      allowParams = flag(c?.allowParams);
      hasOff = typeof c?.offCommand === "string" && c.offCommand.trim() !== "";
    } else {
      const a = (await api.loadAutomations()).find((a) => a.id === target || a.name === target);
      name = a?.name ?? target;
      allowParams = a?.allowParams ?? false;
      hasOff = false;
    }
  }
  const run = async (params: string | null) => {
    try {
      showRun(await api.run(kind, target, params));
    } catch (e) {
      toast(String(e), "error");
    }
  };
  if (!allowParams && !hasOff) return run(null);
  const input = h("input", { class: "input mono", placeholder: hasOff ? t("on, off, or a value") : t("Parameters"), autocomplete: "off" });
  const body = h("form", { class: "dialog-body", onSubmit: (e: Event) => e.preventDefault() }, allowParams ? field(t("Parameters"), input) : null);
  const { close } = dialog(name, body);
  const go = (p: string | null) => {
    close();
    run(p);
  };
  body.append(
    h(
      "div",
      { class: "dialog-actions" },
      hasOff ? button(t("Off"), { icon: "power", onClick: () => go("off") }) : null,
      hasOff ? button(t("On"), { icon: "power", onClick: () => go("on") }) : null,
      button(t("Run"), { kind: "primary", icon: "play", type: "submit", onClick: () => go(input.value || null) }),
    ),
  );
  if (allowParams) input.focus();
}

export async function mountCommands(root: HTMLElement, info: AppInfo): Promise<Cleanup> {
  let commands: Command[] = [];
  let query = "";
  let sortKey: SortKey = "name";
  let ascending = true;
  let loadError: string | null = null;

  const list = h("ul", { class: "command-list" });
  const count = h("span", { class: "count" });
  const search = h("input", {
    class: "input search-input",
    type: "search",
    placeholder: t("Search commands"),
    onInput: () => {
      query = search.value.trim().toLowerCase();
      render();
    },
  });
  const sortSelect = h(
    "select",
    { class: "input select", "aria-label": t("Sort by"), onChange: () => { const [k, d] = sortSelect.value.split(":"); sortKey = k as SortKey; ascending = d !== "desc"; render(); } },
    ...(["name", "command", "ground"] as SortKey[]).flatMap((key) => [
      h("option", { value: `${key}:asc` }, `${t(key === "name" ? "Name" : key === "command" ? "Command" : "Ground")} ↑`),
      h("option", { value: `${key}:desc` }, `${t(key === "name" ? "Name" : key === "command" ? "Command" : "Ground")} ↓`),
    ]),
  );

  root.append(
    h(
      "div",
      { class: "page" },
      h(
        "header",
        { class: "page-head" },
        h("div", { class: "page-title" }, h("h1", null, t("Commands")), count),
        h("div", { class: "head-fill" }),
        button(t("Edit as JSON"), { icon: "braces", kind: "ghost", onClick: () => (location.hash = "commands/json") }),
        button(t("Examples"), { icon: "book", onClick: () => openExamples() }),
        button(t("Add command"), { icon: "plus", kind: "primary", onClick: () => openForm(null) }),
      ),
      h("div", { class: "toolbar" }, h("div", { class: "search" }, icon("search", 16), search), h("label", { class: "sort" }, h("span", null, t("Sort by")), sortSelect)),
      h("div", { class: "scroll" }, list),
    ),
  );

  async function load() {
    try {
      commands = await api.loadCommands();
      loadError = null;
    } catch (e) {
      loadError = String(e);
    }
    render();
  }

  async function save(next: Command[]) {
    try {
      await api.saveCommands(next);
      commands = next;
      render();
      return true;
    } catch (e) {
      toast(t("Not saved: {error}", { error: String(e) }), "error");
      return false;
    }
  }

  function render() {
    count.textContent = t("{n} commands", { n: commands.length });
    if (loadError) {
      list.replaceChildren(h("li", { class: "empty error" }, icon("alert", 20), h("p", null, loadError)));
      return;
    }
    if (!commands.length) {
      list.replaceChildren(h("li", { class: "empty" }, icon("terminal", 28), h("p", { class: "empty-title" }, t("No commands yet.")), h("p", null, t("Add one, or start from an example."))));
      return;
    }
    const rows = commands
      .map((cmd, index) => ({ cmd, index }))
      .filter(({ cmd }) => !query || [cmd.name, cmd.command, cmd.description].some((v) => typeof v === "string" && v.toLowerCase().includes(query)));
    const key = (c: Command) => String(c[sortKey] ?? "").toLowerCase();
    rows.sort((a, b) => key(a.cmd).localeCompare(key(b.cmd)) * (ascending ? 1 : -1));
    if (!rows.length) {
      list.replaceChildren(h("li", { class: "empty" }, h("p", null, t("Nothing matches “{q}”.", { q: search.value }))));
      return;
    }
    list.replaceChildren(...rows.map(({ cmd, index }) => row(cmd, index)));
  }

  function row(cmd: Command, index: number) {
    const params = flag(cmd.allowParams);
    const off = typeof cmd.offCommand === "string" && cmd.offCommand.trim() !== "";
    const background = cmd.ground === "background";
    const emoji = typeof cmd.icon === "string" && cmd.icon ? cmd.icon : null;
    return h(
      "li",
      { class: "command" },
      h("div", { class: "command-icon" }, emoji ? h("span", { class: "emoji" }, emoji) : icon("terminal", 20)),
      h(
        "div",
        { class: "command-body" },
        h("div", { class: "command-name" }, String(cmd.name ?? "")),
        h("code", { class: "command-line" }, String(cmd.command ?? "")),
        typeof cmd.description === "string" && cmd.description ? h("p", { class: "command-note" }, cmd.description) : null,
        h(
          "div",
          { class: "chips" },
          background ? h("span", { class: "chip chip-bg" }, t("background service")) : null,
          params ? h("span", { class: "chip" }, icon("params", 12), t("params"), flag(cmd.quoteParams) ? ' " "' : "") : null,
          off ? h("span", { class: "chip" }, icon("power", 12), String(cmd.offCommand)) : null,
        ),
      ),
      h(
        "div",
        { class: "command-actions" },
        button(null, {
          icon: "play",
          title: background ? t("Runs in the background service") : t("Run"),
          small: true,
          kind: "primary",
          onClick: () => openRunDialog("command", String(cmd.name), { name: String(cmd.name), allowParams: params, hasOff: off }),
        }),
        button(null, { icon: "edit", title: t("Edit"), small: true, onClick: () => openForm(index) }),
        button(null, { icon: "trash", title: t("Delete"), small: true, kind: "danger", onClick: () => confirmDelete(index) }),
      ),
    );
  }

  function confirmDelete(index: number) {
    const cmd = commands[index];
    const body = h("div", { class: "dialog-body" }, h("p", null, t("Automations that use it will stop at that step.")));
    const { close } = dialog(t("Delete {name}?", { name: String(cmd.name) }), body);
    body.append(
      h(
        "div",
        { class: "dialog-actions" },
        button(t("Cancel"), { onClick: close }),
        button(t("Delete"), { kind: "danger", icon: "trash", onClick: async () => { if (await save(commands.filter((_, i) => i !== index))) close(); } }),
      ),
    );
  }

  function openForm(index: number | null, template?: Command) {
    const editing = index === null ? template ?? null : commands[index];
    const value = (k: string) => (editing && typeof editing[k] === "string" ? (editing[k] as string) : "");
    const input = (name: string, placeholder: string, mono = false) =>
      h("input", { class: `input${mono ? " mono" : ""}`, name, value: value(name), placeholder, autocomplete: "off", spellcheck: "false" });
    const name = input("name", t("What you'll call it, e.g. Living room TV"));
    const command = input("command", t("The shell command, e.g. open -a Calculator"), true);
    const offCommand = input("offCommand", t("Runs instead when the parameter is “off”"), true);
    const description = h("textarea", { class: "input", rows: 2, placeholder: t("What it does, for AI assistants and the dashboard (optional)") }, value("description"));
    const canBackground = info.backgroundSupported;
    const ground = h(
      "select",
      { class: "input select", disabled: !canBackground },
      h("option", { value: "foreground" }, t("This app (while you're logged in)")),
      canBackground ? h("option", { value: "background" }, t("Background service (always, as root/system)")) : null,
    );
    ground.value = canBackground ? value("ground") || "foreground" : "foreground";
    const allow = toggle(flag(editing?.allowParams), t("Takes parameters"), () => sync());
    const quote = toggle(flag(editing?.quoteParams), t("Quote them"));
    let chosenIcon = value("icon");
    const picker = emojiPicker(chosenIcon, (e) => (chosenIcon = e));
    const warning = h("p", { class: "form-warning", hidden: true });
    const sync = () => {
      offCommand.disabled = !allow.input.checked;
      quote.input.disabled = !allow.input.checked;
    };
    sync();
    name.addEventListener("input", () => {
      const taken = commands.some((c, i) => i !== index && String(c.name).toLowerCase() === name.value.trim().toLowerCase());
      warning.hidden = !taken;
      warning.textContent = taken ? t("Another command already has this name.") : "";
    });

    // The variable goes where the cursor last was in either command box. Keeping the mouse from
    // taking the focus leaves the cursor there. It stays empty unless the command takes parameters.
    let lastBox = command;
    for (const box of [command, offCommand]) box.addEventListener("focus", () => (lastBox = box));
    const insertVariable = () => {
      if (!allow.input.checked) {
        allow.input.checked = true;
        sync();
      }
      const end = lastBox.value.length;
      lastBox.setRangeText(PARAMS_VARIABLE, lastBox.selectionStart ?? end, lastBox.selectionEnd ?? end, "end");
      lastBox.focus();
    };
    const variable = h("button", { type: "button", class: "token", title: t("Click to put it at the cursor"), onMousedown: (e: MouseEvent) => e.preventDefault(), onClick: insertVariable }, PARAMS_VARIABLE);
    const [beforeVariable, afterVariable] = t("Runs through the shell. Parameters are added at the end, or read them from $CMDTRIGGER_PARAMS.").split(PARAMS_VARIABLE);
    const commandField = field(t("Command"), command, h("span", null, beforeVariable, variable, afterVariable));

    // A name filled in from an app follows the app, until it's typed over.
    let appName = "";
    const apps = appPicker(
      () => command.value.trim(),
      (app) => {
        command.value = app.command;
        if (!name.value.trim() || name.value === appName) {
          name.value = appName = app.name;
          name.dispatchEvent(new Event("input"));
        }
      },
    );
    let chosenMode = false;
    const showApps = (on: boolean) => {
      commandField.hidden = on;
      apps.root.hidden = !on;
      commandTab.classList.toggle("active", !on);
      appTab.classList.toggle("active", on);
      if (on) apps.reveal();
    };
    const mode = (on: boolean) => {
      chosenMode = true;
      showApps(on);
    };
    const commandTab = h("button", { type: "button", class: "active", onClick: () => mode(false) }, t("Command"));
    const appTab = h("button", { type: "button", onClick: () => mode(true) }, t("Open an app"));
    // A command that came from this list opens on it.
    apps.ready.then(() => {
      if (!chosenMode && apps.has(command.value.trim())) showApps(true);
    });
    const form = h(
      "form",
      { class: "dialog-body form-grid", onSubmit: (e: Event) => e.preventDefault() },
      field(t("Name"), name),
      warning,
      h("div", { class: "segmented" }, commandTab, appTab),
      commandField,
      apps.root,
      h("div", { class: "toggles" }, allow.root, quote.root),
      field(t("Off command"), offCommand, t("With parameters on: “off” runs this, “on” runs the command. HomeKit and Home Assistant then show a switch.")),
      field(t("Description"), description),
      field(t("Runs in"), ground, canBackground ? null : t("macOS has no background service; commands run in this app.")),
      field(t("Icon"), picker),
    );
    const { close } = dialog(index === null ? t("Add a command") : t("Edit command"), form, { wide: true });
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
            if (!name.value.trim() || !command.value.trim()) {
              toast(t("A name and a command are required."), "error");
              return;
            }
            const next: Command = { ...(index === null ? {} : editing ?? {}) };
            for (const old of ["trigger", "voice", "voiceReply", "mcpToolDescription"]) delete next[old];
            next.name = name.value.trim();
            next.command = command.value;
            next.offCommand = allow.input.checked ? offCommand.value : "";
            next.ground = ground.value;
            next.allowParams = allow.input.checked;
            next.quoteParams = allow.input.checked && quote.input.checked;
            next.description = description.value.trim();
            next.icon = chosenIcon;
            const list = [...commands];
            if (index === null) list.push(next);
            else list[index] = next;
            if (await save(list)) close();
          },
        }),
      ),
    );
    name.focus();
  }

  async function openExamples() {
    const examples = await api.examples();
    const body = h(
      "div",
      { class: "dialog-body" },
      h("p", { class: "muted" }, t("Ready-made commands for this computer. Pick one to adjust and save it.")),
      h(
        "ul",
        { class: "example-list" },
        ...examples.map((e) =>
          h(
            "li",
            null,
            h(
              "button",
              {
                type: "button",
                class: "example",
                onClick: () => {
                  close();
                  openForm(null, e);
                },
              },
              h("span", { class: "emoji" }, String(e.icon ?? "▶️")),
              h("span", { class: "example-text" }, h("strong", null, String(e.name)), h("code", null, String(e.command)), e.description ? h("span", { class: "muted small" }, String(e.description)) : null),
            ),
          ),
        ),
      ),
    );
    const { close } = dialog(t("Examples"), body, { wide: true });
  }

  await load();
  const unlisten = await on("catalog-changed", () => load());
  return () => unlisten();
}
