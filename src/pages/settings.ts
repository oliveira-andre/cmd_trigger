// Settings: General, iPhone, Server, HomeKit, Home Assistant, and API & AI (webhooks, MCP).
import { api, on, type AppInfo, type BridgeStatus, type LinkStatus, type MqttStatus, type PairingInfo, type PhoneInfo, type ServerInfo } from "../api";
import { button, field, h, toast, toggle } from "../dom";
import { icon, type IconName } from "../icons";
import { t } from "../i18n";
import type { Cleanup } from "../main";

const SECTIONS: [string, IconName, string][] = [
  ["general", "settings", "General"],
  ["iphone", "phone", "iPhone"],
  ["server", "cloud", "Your server"],
  ["homekit", "house", "HomeKit"],
  ["homeassistant", "plug", "Home Assistant"],
  ["api", "bot", "API & AI"],
];

function status(ok: boolean, title: string, detail?: string | null) {
  return h("div", { class: `status-line ${ok ? "ok" : "off"}` }, h("span", { class: "dot" }), h("div", null, h("strong", null, title), detail ? h("p", null, detail) : null));
}

function secret(value: string, placeholder = "") {
  const input = h("input", { class: "input mono", type: "password", value, placeholder, autocomplete: "off", spellcheck: "false" });
  const reveal = h("button", { type: "button", class: "input-adorn", title: t("Show"), onClick: () => { input.type = input.type === "password" ? "text" : "password"; } }, icon("eye", 16));
  return { input, root: h("div", { class: "input-wrap" }, input, reveal) };
}

function copyable(text: string) {
  return h("div", { class: "copyable" }, h("pre", null, text), button(null, { icon: "copy", small: true, kind: "ghost", title: t("Copy"), onClick: () => navigator.clipboard.writeText(text).then(() => toast(t("Copied."), "success")) }));
}

async function saving<T>(btn: HTMLButtonElement, work: () => Promise<T>): Promise<T | undefined> {
  btn.disabled = true;
  try {
    const r = await work();
    toast(t("Saved."), "success");
    return r;
  } catch (e) {
    toast(String(e), "error");
    return undefined;
  } finally {
    btn.disabled = false;
  }
}

export async function mountSettings(root: HTMLElement, info: AppInfo, rest: string[]): Promise<Cleanup> {
  const section = SECTIONS.some(([id]) => id === rest[0]) ? rest[0] : "general";
  const body = h("div", { class: "settings" });
  const cleanups: (() => void)[] = [];
  root.append(
    h(
      "div",
      { class: "page" },
      h("header", { class: "page-head" }, h("div", { class: "page-title" }, h("h1", null, t("Settings")))),
      h(
        "div",
        { class: "settings-layout" },
        h("nav", { class: "subnav" }, ...SECTIONS.map(([id, ic, label]) => h("a", { href: `#settings/${id}`, class: id === section ? "active" : "" }, icon(ic, 16), t(label)))),
        h("div", { class: "scroll" }, body),
      ),
    ),
  );
  if (section === "general") await general(body, info);
  if (section === "iphone") cleanups.push(await iphone(body));
  if (section === "server") cleanups.push(await server(body));
  if (section === "homekit") await homekit(body);
  if (section === "homeassistant") await homeAssistant(body, info);
  if (section === "api") await apiSection(body, info);
  return () => cleanups.forEach((c) => c());
}

async function general(body: HTMLElement, info: AppInfo) {
  const g = await api.generalGet();
  const name = h("input", { class: "input", value: g.computerName, placeholder: g.hostname });
  const autostart = toggle(g.autostart, t("Start CmdTrigger when I log in"));
  const save = button(t("Save"), { kind: "primary", icon: "save" });
  save.addEventListener("click", () => saving(save, () => api.generalSave(name.value, autostart.input.checked)));
  const langs: [string, string][] = [["en", "English"], ["es", "Español"], ["pt", "Português"], ["de", "Deutsch"], ["fr", "Français"], ["jp", "日本語"], ["it", "Italiano"]];
  const language = h("select", { class: "input select", onChange: () => api.setLanguage(language.value) }, ...langs.map(([c, l]) => h("option", { value: c }, l)));
  language.value = info.language;
  body.append(
    h("h2", null, t("General")),
    field(t("Computer name"), name, t("Shown in HomeKit, Home Assistant, your server and AI assistants. Empty: {host}.", { host: g.hostname })),
    autostart.root,
    field(t("Language"), language),
    h("div", { class: "actions" }, save),
    h("h2", null, t("This computer")),
    field(t("MAC addresses (to wake it from another computer)"), h("div", { class: "mono-list" }, ...(g.macAddresses.length ? g.macAddresses : [t("None found")]).map((m) => h("code", null, m)))),
    field(t("Data folder"), h("code", { class: "mono-line" }, info.dataDir)),
  );
}

function ago(ms: number | null): string {
  if (!ms) return t("never");
  const s = Math.max(0, (Date.now() - ms) / 1000);
  if (s < 90) return t("just now");
  if (s < 3600) return t("{n} min ago", { n: Math.round(s / 60) });
  if (s < 86400) return t("{n} h ago", { n: Math.round(s / 3600) });
  return new Date(ms).toLocaleDateString();
}

/** The QR code comes from the Rust side as SVG markup. */
function svgBlock(markup: string): HTMLElement {
  const box = h("div", { class: "qr" });
  box.innerHTML = markup;
  return box;
}

async function iphone(body: HTMLElement): Promise<() => void> {
  let info: PhoneInfo = await api.phoneGet();
  let pairing: PairingInfo | null = info.status?.pairing ?? null;
  let timer = 0;
  const stopPolling = () => { if (timer) { clearInterval(timer); timer = 0; } };
  const poll = () => {
    stopPolling();
    timer = window.setInterval(async () => {
      info = await api.phoneGet();
      pairing = info.status?.pairing ?? null;
      if (!pairing || pairing.pairedWith) stopPolling();
      if (pairing?.pairedWith) toast(t("Paired with {name}.", { name: pairing.pairedWith }), "success");
      draw();
    }, 1000);
  };

  const draw = () => {
    const st = info.status;
    const enabled = toggle(info.enabled, t("Let paired iPhones on this network reach this computer"), async (on) => {
      info = await api.phoneSave(on);
      pairing = null;
      draw();
    });
    const running = st?.running
      ? status(true, t("Ready for iPhones"), info.interfaces.length ? t("On {ips}, port {port}. Your iPhone finds this computer by name on the same Wi-Fi.", { ips: info.interfaces.map((i) => i.ip).join(", "), port: st.port ?? info.port }) : t("No network connection right now."))
      : status(false, info.enabled ? t("Not listening") : t("Off"), st?.error ?? (info.enabled ? null : t("Turn it on to pair an iPhone.")));

    const pairCard = h("div", { class: "pair-card" });
    if (pairing && !pairing.pairedWith) {
      const minutes = Math.floor(pairing.expiresIn / 60);
      const seconds = String(pairing.expiresIn % 60).padStart(2, "0");
      const cancel = button(t("Cancel"), { kind: "ghost", small: true, onClick: async () => { await api.phonePairCancel(); pairing = null; stopPolling(); draw(); } });
      pairCard.append(
        svgBlock(pairing.qrSvg),
        h("div", { class: "pair-text" },
          h("strong", null, t("Scan this with your iPhone's camera")),
          h("p", null, t("It opens the CmdTrigger app and pairs. Or, in the app: Pair a computer → pick this one → type the code.")),
          h("div", { class: "pair-code mono" }, pairing.code),
          h("p", { class: "muted small" }, t("Works once, for {time}.", { time: `${minutes}:${seconds}` })),
          h("div", { class: "actions start" }, cancel)),
      );
    } else if (pairing?.pairedWith) {
      pairCard.append(status(true, t("Paired with {name}", { name: pairing.pairedWith }), t("It can now run this computer's commands and routines, and wake it.")));
    } else {
      const start = button(t("Pair an iPhone"), { kind: "primary", icon: "qr" });
      start.disabled = !st?.running;
      start.addEventListener("click", async () => {
        try {
          pairing = await api.phonePairStart();
          draw();
          poll();
        } catch (e) {
          toast(String(e), "error");
        }
      });
      pairCard.append(h("div", { class: "actions start" }, start));
    }

    const phones = info.phones.length
      ? h("div", { class: "row-list" }, ...info.phones.map((p) =>
          h("div", { class: "row-item" },
            icon("phone", 18),
            h("div", { class: "grow" }, h("strong", null, p.name), h("p", { class: "muted small" }, t("Paired {when} · last seen {seen}", { when: new Date(p.pairedAt).toLocaleDateString(), seen: ago(p.lastSeen) }))),
            button(t("Remove"), { kind: "ghost", small: true, icon: "trash", onClick: async () => {
              if (!confirm(t("Remove {name}? It will need to pair again.", { name: p.name }))) return;
              info = await api.phoneRemove(p.id);
              draw();
            } }))))
      : h("p", { class: "muted" }, t("No iPhone paired yet."));

    body.replaceChildren(
      h("h2", null, t("iPhone")),
      h("p", { class: "lede" }, t("With the CmdTrigger iPhone app you can run this computer's commands and routines from your phone, and wake it from sleep. At home it talks to this computer directly over Wi-Fi, end-to-end encrypted. Away from home it goes through your server.")),
      running,
      enabled.root,
      pairCard,
      h("h2", null, t("Paired iPhones")),
      phones,
    );

    if (info.platform === "macos") {
      const wakeOn = info.wakeForNetwork === true;
      const turnOn = button(t("Turn on (asks for your password)"), { kind: "secondary", icon: "power" });
      turnOn.addEventListener("click", async () => {
        turnOn.disabled = true;
        try {
          info = await api.wakeAccessEnable();
          toast(t("Wake for network access is on."), "success");
          draw();
        } catch (e) {
          if (String(e) !== "cancelled") toast(String(e), "error");
          turnOn.disabled = false;
        }
      });
      body.append(...[
        h("h2", null, t("Waking this Mac")),
        wakeOn
          ? status(true, t("Wake for network access is on"), t("A paired iPhone on this Wi-Fi can wake this Mac from sleep: it sends a wake-up packet straight to the Mac's address."))
          : status(false, t("Wake for network access is off"), t("Without it, a sleeping Mac ignores the iPhone. (System Settings → Battery or Energy → Options.)")),
        wakeOn ? null : h("div", { class: "actions start" }, turnOn),
        h("p", { class: "muted small" }, t("Wake works best with the Mac on power. A shut-down Mac can't be started over the network, only a sleeping one.")),
        info.interfaces.length
          ? field(t("This Mac on the network"), h("div", { class: "mono-list" }, ...info.interfaces.map((i) => h("code", null, `${i.name}  ${i.ip}  ${i.mac}${i.primary ? "  ★" : ""}`))))
          : null,
      ].filter((e): e is HTMLElement => e !== null));
    }
  };
  draw();
  if (pairing && !pairing.pairedWith) poll();
  return () => stopPolling();
}

function linkText(s: LinkStatus | null): [boolean, string, string | null] {
  switch (s?.state) {
    case "online": return [true, t("Connected"), t("Your server can run this computer's commands and automations.")];
    case "connecting": return [false, t("Connecting…"), null];
    case "offline": return [false, t("Can't reach the server"), s.error];
    case "rejected": return [false, t("The server refused this computer"), `${s.message} ${t("Sign in again.")}`];
    default: return [false, t("Not connected"), null];
  }
}

async function server(body: HTMLElement): Promise<() => void> {
  let info: ServerInfo = await api.serverGet();
  const draw = () => {
    body.replaceChildren(
      h("h2", null, t("Your server")),
      h("p", { class: "lede" }, t("Your own CmdTrigger server lets you run this computer's commands from your phone, from anywhere, from iPhone Shortcuts and Siri, from scripts, and from AI assistants. Run it on any machine with Docker (see the README).")),
    );
    if (info.url) {
      const [ok, title, detail] = linkText(info.status);
      const out = button(t("Disconnect"), { kind: "danger", icon: "close" });
      out.addEventListener("click", async () => { info = await api.serverSignOut(); draw(); });
      body.append(
        status(ok, title, detail),
        field(t("Server"), h("div", { class: "inline" }, h("code", { class: "mono-line" }, info.url), button(t("Open dashboard"), { small: true, icon: "external", onClick: () => api.openUrl(info.url) }))),
        field(t("Account"), h("code", { class: "mono-line" }, info.email)),
        h("div", { class: "actions" }, out),
      );
      if (info.status?.state !== "rejected") return;
    }
    const url = h("input", { class: "input mono", value: info.url || "", placeholder: "https://cmd.example.com" });
    const email = h("input", { class: "input", type: "email", value: info.email || "", placeholder: "you@example.com" });
    const password = secret("");
    let create = false;
    const createToggle = toggle(false, t("Create the account (first time on this server)"), (v) => (create = v));
    const go = button(t("Connect"), { kind: "primary", icon: "cloud" });
    go.addEventListener("click", async () => {
      go.disabled = true;
      try {
        info = await api.serverSignIn(url.value, email.value, password.input.value, create);
        toast(t("Connected."), "success");
        draw();
      } catch (e) {
        toast(String(e), "error");
        go.disabled = false;
      }
    });
    body.append(
      h("div", { class: "card-form" },
        field(t("Server address"), url),
        h("div", { class: "row-2" }, field(t("Email"), email), field(t("Password"), password.root)),
        createToggle.root,
        h("p", { class: "muted small" }, t("Your password is only used to register this computer; the app keeps a computer token, not your password.")),
        h("div", { class: "actions" }, go)),
    );
  };
  draw();
  const unlisten = await on<LinkStatus>("server-status", (s) => { info = { ...info, status: s }; draw(); });
  return () => unlisten();
}

async function homekit(body: HTMLElement) {
  let { config, status: st } = await api.homekitGet();
  const name = h("input", { class: "input", value: config.bridgeName });
  const pin = h("input", { class: "input mono", value: config.pin, maxlength: 10 });
  const username = h("input", { class: "input mono", value: config.username, maxlength: 17 });
  const enabled = toggle(config.enabled, t("Turn on the HomeKit bridge"));
  const debug = toggle(config.debugLogging, t("Debug logging"));
  const panel = h("div", { class: "homekit-panel" });
  const draw = (s: BridgeStatus | null) => {
    const code = h("div", { class: "setup-code" }, ...pin.value.split("").map((c) => h("span", { class: c === "-" ? "dash" : "" }, c)));
    const line = s?.running
      ? status(true, t("Running with {n} lights", { n: s.lights }), s.paired ? t("Paired with {n} Home app(s).", { n: s.controllers }) : t("Not paired yet: in the Home app, Add Accessory → More options…, pick the bridge and enter the code."))
      : status(false, t("Off"), s?.error ?? t("Turn it on and save."));
    panel.replaceChildren(h("div", { class: "code-card" }, h("span", { class: "code-label" }, icon("house", 14), t("Setup code")), code), line);
  };
  draw(st);
  pin.addEventListener("input", () => draw(st));
  const save = button(t("Save"), { kind: "primary", icon: "save" });
  save.addEventListener("click", async () => {
    const r = await saving(save, () => api.homekitSave({ enabled: enabled.input.checked, bridgeName: name.value, pin: pin.value, username: username.value, debugLogging: debug.input.checked }));
    if (r) { ({ config, status: st } = r); draw(st); }
  });
  body.append(
    h("h2", null, t("HomeKit")),
    h("p", { class: "lede" }, t("Every command and automation shows up in the Home app as a light, so Siri can run them: “Hey Siri, turn on Calculator”. On runs it; off runs a command's off command; with parameters, brightness becomes the parameter.")),
    panel,
    enabled.root,
    field(t("Bridge name"), name),
    h("div", { class: "row-2" }, field(t("Setup code"), pin), field(t("Bridge ID"), username, t("Changing it makes a new bridge (pair again)."))),
    debug.root,
    h("div", { class: "actions" }, save),
  );
}

async function homeAssistant(body: HTMLElement, info: AppInfo) {
  let { config, status: st } = await api.mqttGet();
  const host = h("input", { class: "input mono", value: config.host, placeholder: "homeassistant.local" });
  const port = h("input", { class: "input mono", type: "number", value: String(config.port), min: 1 });
  const user = h("input", { class: "input", value: config.username, autocomplete: "off" });
  const pass = secret(config.password);
  const prefix = h("input", { class: "input mono", value: config.discoveryPrefix });
  const enabled = toggle(config.enabled, t("Connect to Home Assistant"));
  const line = h("div");
  const draw = (s: MqttStatus | null) =>
    line.replaceChildren(s ? (s.connected ? status(true, t("Connected"), t("{n} entities published under the device “{name}”.", { n: s.entities, name: info.computerName })) : status(false, t("Not connected"), s.error)) : status(false, t("Off")));
  draw(st);
  const save = button(t("Save"), { kind: "primary", icon: "save" });
  save.addEventListener("click", async () => {
    const r = await saving(save, () => api.mqttSave({ enabled: enabled.input.checked, host: host.value.trim(), port: Number(port.value) || 1883, username: user.value, password: pass.input.value, discoveryPrefix: prefix.value.trim() || "homeassistant" }));
    if (r) { ({ config, status: st } = r); draw(st); }
  });
  body.append(
    h("h2", null, t("Home Assistant")),
    h("p", { class: "lede" }, t("Through Home Assistant's MQTT integration, every command appears as a button (and a switch when it has an off command) on a device named after this computer. Use them on dashboards, in automations and with Assist.")),
    line,
    enabled.root,
    h("div", { class: "row-2" }, field(t("MQTT broker"), host, t("Usually your Home Assistant, with the Mosquitto add-on.")), field(t("Port"), port)),
    h("div", { class: "row-2" }, field(t("User"), user, t("A Home Assistant user works.")), field(t("Password"), pass.root)),
    field(t("Discovery prefix"), prefix),
    h("div", { class: "actions" }, save),
  );
}

async function apiSection(body: HTMLElement, info: AppInfo) {
  let data = await api.apiGet();
  const draw = () => {
    const { config, status: st, executable } = data;
    const enabled = toggle(config.enabled, t("Turn on the local API"));
    const lan = toggle(config.allowLan, t("Allow other devices on my network"));
    const port = h("input", { class: "input mono", type: "number", value: String(config.port), min: 1024, max: 65534 });
    const key = secret(config.key);
    key.input.readOnly = true;
    const base = `http://${config.allowLan ? "<this-computer>" : "127.0.0.1"}:${config.port}`;
    const save = button(t("Save"), { kind: "primary", icon: "save" });
    save.addEventListener("click", async () => { const r = await saving(save, () => api.apiSave(enabled.input.checked, Number(port.value), lan.input.checked, false)); if (r) { data = r; draw(); } });
    const regenerate = button(t("New key"), { icon: "refresh" });
    regenerate.addEventListener("click", async () => { const r = await saving(regenerate, () => api.apiSave(config.enabled, config.port, config.allowLan, true)); if (r) { data = r; draw(); } });
    const mcp = JSON.stringify({ mcpServers: { cmdtrigger: { command: executable, args: ["mcp"] } } }, null, 2);
    body.replaceChildren(
      h("h2", null, t("AI assistants (MCP)")),
      h("p", { class: "lede" }, t("Claude and other AI assistants can run this computer's commands and automations as tools. The command's description tells the AI what it does.")),
      field(t("Claude Code"), copyable(`claude mcp add cmdtrigger -- "${executable}" mcp`)),
      field(t("Claude Desktop (claude_desktop_config.json)"), copyable(mcp)),
      h("p", { class: "muted small" }, t("For all your computers at once, use your server's MCP address instead (Settings → Your server → dashboard → API & integrations).")),
      h("h2", null, t("Local API and webhooks")),
      st?.running ? status(true, t("Listening on {addr}", { addr: st.address ?? "" })) : status(false, t("Not running"), st?.error),
      enabled.root,
      lan.root,
      field(t("Port"), port, info.backgroundSupported ? t("The background service uses the next port.") : null),
      field(t("API key"), h("div", { class: "inline" }, key.root, regenerate)),
      h("div", { class: "actions" }, save),
      field(t("Run something"), copyable(`curl -X POST ${base}/api/v1/run -H "Authorization: Bearer ${config.key}" \\\n  -H "Content-Type: application/json" -d '{"target": "Calculator"}'`)),
      h("p", { class: "muted small" }, t("Webhooks need no key: add a Webhook trigger to an automation and call its address.")),
    );
  };
  draw();
}
