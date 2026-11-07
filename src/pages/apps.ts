// The "Open an app" tab of the command form: everything installed, searchable. Picking one fills
// in the line that opens it, so what's saved is an ordinary command.
import { api, type InstalledApp } from "../api";
import { h } from "../dom";
import { icon } from "../icons";
import { t } from "../i18n";
import { fold } from "./emoji";

/// `current` is the command line as it stands; a row lights up when it's the one that row runs.
export function appPicker(current: () => string, onPick: (app: InstalledApp) => void) {
  let apps: InstalledApp[] = [];
  let shown: InstalledApp[] = [];

  const rows = () => [...list.querySelectorAll<HTMLElement>(".app-row")];
  const line = h("code", { class: "mono-line" });

  const mark = () => {
    rows().forEach((row, i) => row.classList.toggle("active", shown[i]?.command === current()));
    line.textContent = current();
  };

  const pick = (app: InstalledApp | undefined) => {
    if (!app) return;
    onPick(app);
    mark();
  };

  const draw = () => {
    const query = fold(input.value.trim());
    const hits = apps.filter((a) => fold(a.name).includes(query));
    const starts = (a: InstalledApp) => (fold(a.name).startsWith(query) ? 0 : 1);
    shown = hits.sort((a, b) => starts(a) - starts(b));
    if (!shown.length) {
      const note = apps.length ? t("Nothing matches “{q}”.", { q: input.value }) : t("No apps found on this computer.");
      list.replaceChildren(h("p", { class: "muted small" }, note));
    } else {
      list.replaceChildren(...shown.map((a, i) => h("button", { type: "button", class: "app-row", tabindex: i ? -1 : 0, onClick: () => pick(a) }, a.name)));
    }
    mark();
  };

  const input = h("input", {
    class: "input search-input",
    placeholder: t("Search apps"),
    "aria-label": t("Search apps"),
    autocomplete: "off",
    spellcheck: "false",
    onInput: draw,
    onKeydown: (e: KeyboardEvent) => {
      if (e.key === "Enter") {
        // Enter would otherwise save the whole form.
        e.preventDefault();
        if (input.value.trim()) pick(shown[0]);
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        rows()[0]?.focus();
      } else if (e.key === "Escape" && input.value) {
        // The first Escape clears the search; the next one closes the dialog as usual.
        e.stopPropagation();
        input.value = "";
        draw();
      }
    },
  });

  // The rows are one Tab stop, so Tab can still get past them to Save.
  const list = h("div", {
    class: "app-list",
    onKeydown: (e: KeyboardEvent) => {
      const step = ({ ArrowUp: -1, ArrowDown: 1 } as Record<string, number>)[e.key];
      if (!step) return;
      e.preventDefault();
      const all = rows();
      const next = all[all.indexOf(document.activeElement as HTMLElement) + step];
      if (next) next.focus();
      else if (step < 0) input.focus();
    },
  });

  const root = h("div", { class: "app-picker", hidden: true }, h("div", { class: "search" }, icon("search", 16), input), list, line);
  const ready = api
    .installedApps()
    .catch(() => [])
    .then((found) => {
      apps = found;
      draw();
    });

  return {
    root,
    ready,
    has: (command: string) => apps.some((a) => a.command === command),
    /// Call once it's on screen: bring the chosen app into view.
    reveal: () => {
      mark();
      const chosen = list.querySelector<HTMLElement>(".app-row.active");
      if (chosen) list.scrollTop = chosen.offsetTop - list.clientHeight / 2;
    },
  };
}
