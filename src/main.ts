// The app shell: a sidebar (Commands, Automations, Activity, Settings) and one view, picked by
// the URL hash. The tray navigates with a "navigate" event.
import "./fonts";
import "./styles.css";
import { api, on, type AppInfo, type LinkStatus } from "./api";
import { h, mark } from "./dom";
import { icon, type IconName } from "./icons";
import { setLang, t } from "./i18n";
import { mountActivity } from "./pages/activity";
import { mountAutomations } from "./pages/automations";
import { mountCommands, openRunDialog } from "./pages/commands";
import { mountSettings } from "./pages/settings";
import { mountText } from "./pages/text";

export type Cleanup = () => void;
type Mount = (root: HTMLElement, info: AppInfo, rest: string[]) => Promise<Cleanup | void>;

const SECTIONS: [string, IconName, string, Mount][] = [
  ["commands", "terminal", "Commands", mountCommands],
  ["automations", "zap", "Automations", mountAutomations],
  ["activity", "activity", "Activity", mountActivity],
  ["settings", "settings", "Settings", mountSettings],
];

let cleanup: Cleanup | void;
let info: AppInfo;
const serverChip = h("button", { class: "server-chip", type: "button", onClick: () => go("settings/server") });

export function go(route: string) {
  if (location.hash.slice(1) === route) render();
  else location.hash = route;
}

function drawServer(status: LinkStatus | null) {
  const state = status?.state ?? "off";
  const text = { online: "Server connected", connecting: "Connecting…", offline: "Server offline", rejected: "Sign in again", off: "No server" }[state];
  serverChip.className = `server-chip ${state}`;
  serverChip.replaceChildren(h("span", { class: "dot" }), t(text));
}

async function render() {
  info = await api.appInfo();
  setLang(info.language);
  const [section, ...rest] = (location.hash.slice(1) || "commands").split("/");
  cleanup?.();
  cleanup = undefined;
  const view = h("section", { class: "view" });
  const nav = h(
    "nav",
    { class: "nav" },
    ...SECTIONS.map(([id, ic, label]) =>
      h("a", { href: `#${id}`, class: `nav-item${(section === id || (section === "run" && id === "commands")) ? " active" : ""}` }, icon(ic, 18), h("span", null, t(label))),
    ),
  );
  document.getElementById("overlays")!.replaceChildren();
  document.getElementById("app")!.replaceChildren(
    h(
      "div",
      { class: "shell" },
      h(
        "aside",
        { class: "sidebar" },
        h("div", { class: "brand" }, mark(), h("div", null, h("strong", null, "CmdTrigger"), h("span", { class: "muted small" }, info.computerName))),
        nav,
        h("div", { class: "sidebar-foot" }, serverChip, h("span", { class: "muted small" }, `v${info.version}`)),
      ),
      view,
    ),
  );
  api.serverGet().then((s) => drawServer(s.status)).catch(() => drawServer(null));
  if (section === "run") {
    await mountCommands(view, info);
    const [kind, ...id] = rest;
    openRunDialog(kind === "automation" ? "automation" : "command", decodeURIComponent(id.join("/")));
    return;
  }
  if (section === "commands" && rest[0] === "json") {
    cleanup = await mountText(view, info);
    return;
  }
  const mount = SECTIONS.find(([id]) => id === section)?.[3] ?? mountCommands;
  cleanup = await mount(view, info, rest);
}

window.addEventListener("hashchange", () => render());
on<string>("navigate", (route) => go(route));
on("language-changed", () => render());
on<LinkStatus>("server-status", (s) => drawServer(s));
render();
