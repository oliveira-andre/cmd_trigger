// Activity: every run, from every source, newest first, live.
import { api, on, type RunRecord } from "../api";
import { button, dialog, h, toast } from "../dom";
import { icon, type IconName } from "../icons";
import { t } from "../i18n";
import type { Cleanup } from "../main";

const SOURCE_ICON: Record<string, IconName> = {
  app: "terminal", tray: "terminal", homekit: "house", mqtt: "house", webhook: "webhook", api: "plug", mcp: "bot", cli: "terminal",
  schedule: "clock", startup: "sunrise", wake: "power", network: "wifi", usb: "usb", display: "monitor", file: "file",
};

function when(ms: number) {
  const d = new Date(ms);
  const today = new Date().toDateString() === d.toDateString();
  return today ? d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" }) : d.toLocaleString();
}

export async function mountActivity(root: HTMLElement): Promise<Cleanup> {
  let runs: RunRecord[] = [];
  const list = h("ul", { class: "runs" });
  const clear = button(t("Clear"), { icon: "trash", onClick: () => confirmClear() });
  root.append(
    h(
      "div",
      { class: "page" },
      h("header", { class: "page-head" }, h("div", { class: "page-title" }, h("h1", null, t("Activity")), h("span", { class: "count" }, t("Every run, from everywhere"))), h("div", { class: "head-fill" }),
        clear,
        button(t("Refresh"), { icon: "refresh", onClick: () => load() })),
      h("div", { class: "scroll" }, list),
    ),
  );

  function item(r: RunRecord) {
    const ok = r.status === "succeeded";
    const source = r.source.startsWith("server") ? "cloud" : SOURCE_ICON[r.source] ?? "zap";
    const out = h("pre", { class: "output", hidden: true }, r.output || t("(no output)"));
    return h(
      "li",
      { class: "run", onClick: () => (out.hidden = !out.hidden) },
      h(
        "div",
        { class: "run-head" },
        h("span", { class: `status-dot ${ok ? "ok" : "bad"}` }),
        h("strong", null, r.name),
        r.kind === "automation" ? h("span", { class: "chip" }, icon("zap", 12), t("automation")) : null,
        r.params ? h("code", { class: "chip mono" }, r.params) : null,
        h("span", { class: "head-fill" }),
        h("span", { class: "chip" }, icon(source as IconName, 12), r.source),
        !ok && r.exitCode !== null ? h("span", { class: "chip bad" }, `exit ${r.exitCode}`) : null,
        h("span", { class: "muted small mono" }, `${when(r.at)} · ${r.durationMs < 1000 ? `${r.durationMs} ms` : `${(r.durationMs / 1000).toFixed(1)} s`}`),
      ),
      out,
    );
  }

  function confirmClear() {
    const body = h("div", { class: "dialog-body" }, h("p", null, t("This removes every run and its output from this computer. It can't be undone.")));
    const { close } = dialog(t("Clear activity?"), body);
    body.append(
      h(
        "div",
        { class: "dialog-actions" },
        button(t("Cancel"), { onClick: close }),
        button(t("Clear"), {
          kind: "danger",
          icon: "trash",
          onClick: async () => {
            try {
              await api.clearHistory();
              runs = [];
              render();
              close();
            } catch (e) {
              toast(String(e), "error");
            }
          },
        }),
      ),
    );
  }

  function render() {
    clear.disabled = !runs.length;
    if (!runs.length) {
      list.replaceChildren(h("li", { class: "empty" }, icon("activity", 28), h("p", { class: "empty-title" }, t("Nothing has run yet.")), h("p", null, t("Runs from the app, HomeKit, Home Assistant, webhooks, AI assistants, your server and automations show up here."))));
      return;
    }
    list.replaceChildren(...runs.map(item));
  }

  async function load() {
    try {
      runs = await api.history(300);
    } catch {
      runs = [];
    }
    render();
  }

  await load();
  const unlisten = await on<RunRecord>("run", (r) => {
    runs.unshift(r);
    clear.disabled = false;
    list.prepend(item(r));
    list.querySelector(".empty")?.remove();
  });
  return () => unlisten();
}
