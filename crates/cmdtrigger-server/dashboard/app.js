// The CmdTrigger dashboard: sign in, tap to run things on your computers, see what ran, and
// make API tokens for iPhone Shortcuts, scripts and AI assistants. Plain JS, no build step.
"use strict";

const API = "/api/v1";
const store = {
  get token() { try { return localStorage.getItem("ct.session") || ""; } catch { return ""; } },
  set token(v) { try { v ? localStorage.setItem("ct.session", v) : localStorage.removeItem("ct.session"); } catch {} },
};
let tab = "computers";
let refreshTimer = null;

function h(tag, attrs, ...children) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v === null || v === undefined || v === false) continue;
    if (k === "class") el.className = v;
    else if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else if (k === "value") el.value = v;
    else el.setAttribute(k, v === true ? "" : v);
  }
  for (const c of children.flat()) if (c !== null && c !== undefined && c !== false) el.append(c instanceof Node ? c : String(c));
  return el;
}

async function api(method, path, body) {
  const res = await fetch(API + path, {
    method,
    headers: { "content-type": "application/json", ...(store.token ? { authorization: "Bearer " + store.token } : {}) },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (res.status === 401 && path !== "/login" && path !== "/signup") {
    store.token = "";
    render();
    throw new Error("Signed out");
  }
  const text = await res.text();
  const data = text ? JSON.parse(text) : null;
  if (!res.ok) throw new Error((data && data.error) || res.statusText);
  return data;
}

function toast(message, kind = "") {
  const el = h("div", { class: "toast " + kind }, message);
  document.getElementById("toasts").append(el);
  setTimeout(() => el.remove(), 3500);
}

function sheet(title, ...body) {
  const host = document.getElementById("overlay");
  const close = () => host.replaceChildren();
  const backdrop = h("div", { class: "backdrop", onclick: (e) => e.target === backdrop && close() }, h("div", { class: "sheet", role: "dialog" }, h("h2", null, title), ...body));
  host.replaceChildren(backdrop);
  return close;
}

function ago(seconds) {
  if (!seconds) return "never";
  const d = Math.max(0, Date.now() / 1000 - seconds);
  if (d < 60) return "just now";
  if (d < 3600) return Math.floor(d / 60) + " min ago";
  if (d < 86400) return Math.floor(d / 3600) + " h ago";
  return new Date(seconds * 1000).toLocaleDateString();
}

const statusChip = (s) => h("span", { class: "chip " + (s === "succeeded" ? "ok" : s === "queued" || s === "running" ? "wait" : "bad") }, s.replace("_", " "));

// ---------- sign in ----------

function renderAuth(root, creating = false) {
  const email = h("input", { type: "email", autocomplete: "username", placeholder: "you@example.com" });
  const password = h("input", { type: "password", autocomplete: creating ? "new-password" : "current-password", placeholder: creating ? "At least 8 characters" : "" });
  const error = h("p", { class: "error", hidden: true });
  const submit = h("button", { class: "btn primary", type: "submit" }, creating ? "Create account" : "Sign in");
  const form = h(
    "form",
    {
      class: "auth",
      onsubmit: async (e) => {
        e.preventDefault();
        submit.disabled = true;
        try {
          const device = (navigator.userAgent.includes("iPhone") ? "iPhone" : navigator.userAgent.includes("Android") ? "Android" : "Browser") + " dashboard";
          const s = await api("POST", creating ? "/signup" : "/login", { email: email.value, password: password.value, device });
          store.token = s.token;
          render();
        } catch (err) {
          error.hidden = false;
          error.textContent = err.message;
          submit.disabled = false;
        }
      },
    },
    h("div", { class: "brand" }, h("img", { src: "/icon.svg", alt: "" }), h("h1", null, "CmdTrigger")),
    h("p", { class: "lede" }, creating ? "Create the account your computers will sign in with." : "Run commands on your computers from anywhere."),
    h("label", null, "Email", email),
    h("label", null, "Password", password),
    error,
    submit,
    h("button", { type: "button", class: "switch", onclick: () => { root.replaceChildren(); renderAuth(root, !creating); } }, creating ? "I already have an account" : "Create an account"),
  );
  root.append(form);
  email.focus();
}

// ---------- computers ----------

async function runTile(computer, entry, tile) {
  const go = async (params) => {
    tile.classList.add("busy");
    try {
      const run = await api("POST", "/run", { computer: computer.id, target: entry.id, kind: entry.kind, params, wait: 20 });
      const ok = run.status === "succeeded";
      toast(`${entry.name}: ${run.status.replace("_", " ")}`, ok ? "ok" : run.status === "queued" || run.status === "running" ? "" : "bad");
      if (run.output && run.output.trim()) {
        sheet(entry.name, statusChip(run.status), h("pre", null, run.output), h("div", { class: "actions" }, h("button", { class: "btn", onclick: () => document.getElementById("overlay").replaceChildren() }, "Close")));
      }
    } catch (err) {
      toast(err.message, "bad");
    } finally {
      tile.classList.remove("busy");
    }
  };
  if (!entry.allowParams && !entry.hasOff) return go(null);
  const input = h("input", { placeholder: entry.hasOff ? "on, off, or a value" : "Parameters", autocomplete: "off" });
  const close = sheet(
    entry.name,
    entry.description ? h("p", { class: "lede" }, entry.description) : null,
    entry.allowParams ? h("label", null, "Parameters", input) : null,
    h(
      "div",
      { class: "actions" },
      entry.hasOff ? h("button", { class: "btn", onclick: () => { close(); go("off"); } }, "Off") : null,
      entry.hasOff ? h("button", { class: "btn", onclick: () => { close(); go("on"); } }, "On") : null,
      h("button", { class: "btn primary", onclick: () => { close(); go(input.value || null); } }, "Run"),
    ),
  );
  if (entry.allowParams) input.focus();
}

async function renderComputers(main) {
  const computers = await api("GET", "/computers");
  if (!computers.length) {
    main.append(
      h(
        "div",
        { class: "panel" },
        h("h2", null, "No computers yet"),
        h("p", { class: "lede" }, "Open the CmdTrigger app on a computer → Settings → Server, and sign in with this account. Or, on a computer without a screen:"),
        h("pre", null, `cmdtrigger connect ${location.origin} --email YOU@EXAMPLE.COM`),
      ),
    );
    return;
  }
  for (const c of computers) {
    const online = c.online.length > 0;
    const tiles = c.catalog.map((e) => {
      const available = c.online.includes(e.ground);
      const tile = h(
        "button",
        { class: "tile", disabled: !available, title: available ? e.description || e.name : `${c.name}'s ${e.ground} agent is offline`, onclick: () => runTile(c, e, tile) },
        h("span", { class: "icon" }, e.icon || (e.kind === "automation" ? "⚙️" : "▶️")),
        h("span", { class: "name" }, e.name),
        h("span", { class: "kind" }, e.kind === "automation" ? "automation" : e.ground === "background" ? "background" : "command"),
      );
      return tile;
    });
    main.append(
      h(
        "section",
        { class: "computer" },
        h(
          "div",
          { class: "computer-head" },
          h("span", { class: "dot" + (online ? " on" : "") }),
          h("h2", null, c.name),
          h("span", { class: "meta" }, online ? `online${c.online.length > 1 ? " · tray + service" : c.online[0] === "background" ? " · service" : ""}` : `offline · seen ${ago(c.lastSeen)}`),
          h("span", { class: "fill" }),
          h("button", {
            class: "btn ghost small danger",
            onclick: async () => {
              if (!confirm(`Remove ${c.name}? Its app will need to sign in again.`)) return;
              await api("DELETE", "/computers/" + c.id);
              render();
            },
          }, "Remove"),
        ),
        tiles.length ? h("div", { class: "tiles" }, tiles) : h("p", { class: "empty" }, "Nothing published yet. Add commands in the app."),
      ),
    );
  }
}

// ---------- history ----------

async function renderHistory(main) {
  const runs = await api("GET", "/runs?limit=100");
  if (!runs.length) return main.append(h("p", { class: "empty" }, "Nothing has run from here yet."));
  main.append(
    h(
      "ul",
      { class: "runs" },
      runs.map((r) => {
        const out = h("pre", { hidden: true }, r.output || "(no output)");
        return h(
          "li",
          { class: "run", onclick: () => (out.hidden = !out.hidden) },
          h(
            "div",
            { class: "run-head" },
            statusChip(r.status),
            h("strong", null, r.targetName),
            h("span", { class: "meta" }, `${r.computerName}${r.params ? ` · "${r.params}"` : ""} · ${r.source} · ${ago(r.createdAt)}`),
            r.exitCode !== null && r.exitCode !== 0 ? h("span", { class: "chip bad" }, "exit " + r.exitCode) : null,
          ),
          out,
        );
      }),
    ),
  );
}

// ---------- tokens & integrations ----------

function recipes(token) {
  const origin = location.origin;
  return [
    h("h2", null, "iPhone Shortcuts and Siri"),
    h(
      "ol",
      null,
      h("li", null, "Shortcuts → new shortcut → add “Get Contents of URL”."),
      h("li", null, `URL: ${origin}${API}/run · Method: POST`),
      h("li", null, `Headers: Authorization = Bearer ${token}`),
      h("li", null, "Request Body: JSON with computer, target (and params, wait: 20)."),
      h("li", null, "Name the shortcut what you'll say: “Hey Siri, <name>”."),
    ),
    h("h2", null, "Scripts"),
    h("pre", null, `curl -X POST ${origin}${API}/run \\\n  -H "Authorization: Bearer ${token}" \\\n  -H "Content-Type: application/json" \\\n  -d '{"computer": "Den PC", "target": "Calculator", "wait": 20}'`),
    h("h2", null, "AI assistants (MCP)"),
    h("pre", null, `claude mcp add --transport http cmdtrigger ${origin}${API}/mcp \\\n  --header "Authorization: Bearer ${token}"`),
    h("h2", null, "Home Assistant"),
    h("pre", null, `rest_command:\n  den_pc_calculator:\n    url: ${origin}${API}/run\n    method: POST\n    headers:\n      authorization: "Bearer ${token}"\n    content_type: application/json\n    payload: '{"computer": "Den PC", "target": "Calculator"}'`),
  ];
}

async function renderTokens(main) {
  const tokens = await api("GET", "/tokens");
  const name = h("input", { placeholder: "Name, e.g. iPhone Shortcuts" });
  main.append(
    h(
      "div",
      { class: "panel" },
      h("h2", null, "API tokens"),
      h("p", { class: "lede" }, "For iPhone Shortcuts, scripts, Home Assistant and AI assistants. A token can run things and read history, but can't change your account."),
      h(
        "div",
        { class: "row" },
        name,
        h("button", {
          class: "btn primary",
          onclick: async () => {
            const t = await api("POST", "/tokens", { name: name.value });
            sheet(
              `Token “${t.name}”`,
              h("p", { class: "lede" }, "Copy it now; it won't be shown again."),
              h("pre", null, t.token),
              h("div", { class: "actions" }, h("button", { class: "btn", onclick: () => navigator.clipboard?.writeText(t.token).then(() => toast("Copied", "ok")) }, "Copy"), h("button", { class: "btn primary", onclick: () => { document.getElementById("overlay").replaceChildren(); render(); } }, "Done")),
              ...recipes(t.token),
            );
          },
        }, "Create token"),
      ),
      tokens.length
        ? tokens.map((t) =>
            h(
              "div",
              { class: "list-item" },
              h("div", { class: "fill" }, h("strong", null, t.name), h("div", { class: "meta" }, `created ${ago(t.createdAt)} · used ${ago(t.lastUsedAt)}`)),
              h("button", { class: "btn danger small", onclick: async () => { if (confirm(`Delete “${t.name}”? Anything using it stops working.`)) { await api("DELETE", "/tokens/" + t.id); render(); } } }, "Delete"),
            ),
          )
        : h("p", { class: "meta" }, "No tokens yet."),
    ),
    h("div", { class: "panel" }, ...recipes("YOUR_TOKEN")),
  );
}

// ---------- shell ----------

async function render() {
  clearTimeout(refreshTimer);
  const root = document.getElementById("app");
  root.replaceChildren();
  if (!store.token) return renderAuth(root);
  let me;
  try {
    me = await api("GET", "/me");
  } catch {
    return;
  }
  const tabs = [["computers", "Computers"], ["history", "History"], ["tokens", "API & integrations", "API"]];
  const main = h("main");
  root.append(
    h(
      "header",
      { class: "top" },
      h("img", { src: "/icon.svg", alt: "" }),
      h("h1", null, "CmdTrigger"),
      h("nav", { class: "tabs" }, tabs.map(([id, label, short]) => h("button", { class: tab === id ? "active" : "", onclick: () => { tab = id; render(); } }, short ? [h("span", { class: "long" }, label), h("span", { class: "short" }, short)] : label))),
      h("span", { class: "fill" }),
      h("button", { class: "btn ghost small", title: me.email, onclick: async () => { await api("POST", "/logout").catch(() => {}); store.token = ""; render(); } }, "Sign out"),
    ),
    main,
  );
  try {
    if (tab === "computers") await renderComputers(main);
    if (tab === "history") await renderHistory(main);
    if (tab === "tokens") await renderTokens(main);
  } catch (err) {
    main.append(h("p", { class: "error" }, err.message));
  }
  // Keep online dots and history fresh (not while a sheet is open).
  if (tab !== "tokens") {
    refreshTimer = setTimeout(() => { if (!document.getElementById("overlay").children.length) render(); else refreshTimer = setTimeout(render, 5000); }, 10000);
  }
}

render();
