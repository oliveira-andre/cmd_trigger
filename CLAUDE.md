# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

CmdTrigger is the owner's own command and automation system. It started as a Rust rebuild of
the TRIGGERcmd agent (notes in `docs/triggercmd-reference.md`). The owner wants **no TRIGGERcmd
account or cloud**, so nothing may talk to triggercmd.com.

- **Agent**: the Tauri tray app, or the `cmdtrigger` CLI. It runs commands and automations.
  Ways in: HomeKit, Home Assistant (MQTT discovery), a local API with webhooks, MCP, schedules
  and system events.
- **iPhone app** (`ios/`, SwiftUI, iOS 17): runs the computers' commands and routines, wakes a
  sleeping Mac, and offers Siri/Shortcuts actions. At home it uses the phone link (direct,
  sealed); away from home it uses the server with an API token.
- **Server** (`cmdtrigger-server`): self-hosted with Docker + Caddy. It relays runs from a phone
  dashboard, from API tokens (iPhone Shortcuts) and from MCP over HTTP. Agents connect to it
  by WebSocket.

## Commands

Rust lives in `~/.cargo/bin` (`export PATH="$HOME/.cargo/bin:$PATH"`).

```sh
npm install
npm run tauri dev                         # the app; Vite on :1450
npm run dev:scratch                       # the app with data in ./.dev-data
npm run server                            # server on 127.0.0.1:8080 (CMDTRIGGER_ADDR/DB/ALLOW_SIGNUP)
cargo run -p cmdtrigger-cli -- --help     # headless agent and tools
npx tsc --noEmit
npx tauri build --debug --bundles app     # debug CmdTrigger.app

cargo test --workspace
cargo test -p cmdtrigger-core --lib                 # runner, engine, local API, MQTT layout, history…
cargo test -p cmdtrigger-core --test homekit        # HAP pairing/session with a test controller
cargo test -p cmdtrigger-server                     # real agent ⇄ real server end to end (Unix)
cargo clippy --workspace --all-targets              # zero warnings
cargo fmt --all
npm run check:linux      # Docker: fmt, clippy -D warnings, tests, .deb, server + app + service smoke run
npm run check:windows    # Docker: clippy for windows-gnu, cmdtrigger.exe, core tests under Wine 10
ios/ui-test.sh           # simulator UI tests against the debug .app + server on scratch data (ports 7380/7391/8093)
xcodebuild -project ios/CmdTrigger.xcodeproj -scheme CmdTrigger -destination 'platform=iOS Simulator,name=iPhone 17' -only-testing:CmdTriggerTests test
PHONE_VECTORS=1 cargo test -p cmdtrigger-core phone::crypto   # rewrite ios/CmdTriggerTests/phone_vectors.json
ios/build-ipa.sh --install                                    # the user's iPhone over USB (free team: 7-day installs)
```

To try the real app without touching real data, use a scratch folder:
`open -n -g --env CMDTRIGGER_DATA_DIR=<scratch> target/debug/bundle/macos/CmdTrigger.app`.
- Pre-create `commands.json` there, so the window doesn't open as a first run.
- Give it its own `api.json` port, so it doesn't collide with a running app.

## Architecture

- **`crates/cmdtrigger-protocol`**: the contract.
  - `CatalogEntry`, the agent WebSocket messages (`AgentMessage`/`ServerMessage`), the REST
    types (endpoint table in the module doc), and `slug()`.
  - `mcp`: JSON-RPC dispatch and `tool_for`.
  - Agents publish catalogs (names, icons, descriptions), never command lines.
- **`crates/cmdtrigger-core`**:
  - **Storage and model:**
    - `paths::DataDir` holds the files: `commands.json`, `automations.json`, `history.jsonl`,
      `homekit.json`, `mqtt.json`, `api.json`, `server.json`, `settings.json`, `debug.log`,
      and the `homekit/` folder.
    - `store` writes atomically (`write_private` is 0600 for secrets). It seeds the defaults,
      imports `~/.TRIGGERcmdData/commands.json` once, and holds the examples.
    - `model::Command` is a JSON map with accessors. `normalize()` converts TRIGGERcmd field
      names.
  - **Running:**
    - `exec` builds the command line (params, off/on, quoting, HomeKit brightness) and runs the
      shell.
    - `runner::Runner` runs every command and automation from any source: ground check, env
      vars, history, broadcast. `trigger()` applies the conditions; `run()` (manual) skips them.
  - **Automations:**
    - `automation` holds the model and `validate`.
    - `engine` runs a 5 s tick (schedules, intervals, wake). It polls the monitors every 10 s,
      only when some automation uses them, and watches files.
    - `monitors` reads:
      - the network: the SSID, or the router's MAC where macOS redacts SSIDs;
      - USB device names: `ioreg`, `/sys`, PowerShell;
      - the display count: CoreGraphics, `/sys/class/drm`, WMI;
      - wake from sleep: `WakeDetector`.
  - **Ways in:**
    - `homekit/` is the native HAP bridge; the catalog becomes lights. Storage uses
      hap-nodejs's file layout. UUID seeds are `cmdtrigger:command:<id>`,
      `cmdtrigger:automation:<id>` and `cmdtrigger-bridge:<username>`.
    - `mqtt` publishes Home Assistant discovery (buttons, switches, texts), with LWT
      availability and stale-config cleanup.
    - `local_api` is axum on 7171 (the background service uses +1). It serves run, catalog,
      runs and `/hooks/<id>`.
    - `mcp_stdio` (`cmdtrigger mcp`) proxies to the local API.
    - `link` is the server client. `sign_in` registers the computer and keeps only the agent
      token. Then it holds a WebSocket: Hello/Catalog/Result out, Run/Pong/Rejected in.
  - **Phone link** (`phone/`):
    - `crypto` handles pairing proofs and HKDF, plus ChaCha20-Poly1305 sealing (vectors shared
      with Swift).
    - `mod.rs` holds `phones.json` (0600), the pairing window, the axum listener on 7180 and the
      ops (status/run/runs/wakeScreen/unpair).
    - `bonjour` advertises through mDNSResponder on macOS (FFI, so a sleep proxy can keep the
      record) and through mdns-sd elsewhere.
    - `host` covers interfaces, `pmset` womp and waking the screen.
    - Only the foreground agent serves it. The local API's `/api/v1/pair` is what
      `cmdtrigger pair` uses.
  - **Assembly:**
    - `agent::Agent` wires everything, with a `restart_*` per subsystem.
    - `headless` is the CLI.
    - `service` installs the systemd unit or Windows service; `winservice` is the SCM entry
      (`cfg(windows)`).
- **`crates/cmdtrigger-server`** (axum + rusqlite):
  - **Tables:** users, tokens (`session` | `api`), computers (catalog JSON per ground), and runs
    (the last 1000 per user).
  - **Agents** are kept in memory, keyed by (computer, ground). `start_run` resolves the
    computer and target, sends a Run, and can wait on a watch channel.
  - **Rules:** only the first account can sign up by default (`Signup::First`). Logins lock
    after 5 failures.
  - **`dashboard/`** is static HTML/JS/CSS, embedded with `include_str!`.
- **`src-tauri`**:
  - `lib.rs` starts the agent at launch, with no sign-in. On first run the window opens and, in
    release builds, autostart is enabled. It forwards `run`, `catalog-changed` and
    `server-status` events to the window.
  - `tray.rs` builds the Run submenu, the server line, Background Service and Language.
  - `windows.rs` holds the single `main` window with hash routes; `commands.rs` is the IPC.
- **`ios/`**: `CmdTrigger.xcodeproj` is hand-written, with file-system synchronized groups
  (copied from Thingscan), so new files in the folders build automatically. Bundle
  `app.cmdtrigger.ios`, team ZT23B58Z4D.
  - `Model/AppModel.swift` (@Observable, `AppModel.shared`, also used by App Intents) merges
    paired computers and server computers into `ComputerCard`s. It routes runs over Wi-Fi first,
    then the server.
  - `Net/` holds `PhoneLink` (pairing and sealed calls), `PhoneCrypto`, `ServerAPI`, and `Wake`
    (unicast magic packets plus a TCP knock; iOS forbids broadcast without Apple's multicast
    entitlement).
  - `CT_RESET=1` in the launch environment clears all state (the UI tests use it).
- **`src/`** (vanilla TS):
  - `main.ts` is the shell: the sidebar and the router (`#commands`, `#commands/json`,
    `#automations[/id]`, `#activity`, `#settings/<section>`, `#run/<kind>/<id>`).
  - The pages are in `pages/`. `api.ts` holds the typed commands; `i18n.ts` falls back to
    English.

## Contracts

- **No TRIGGERcmd.** Don't add triggercmd.com calls or a TRIGGERcmd sign-in.
- **Ground.** The app is the foreground agent and the service the background one. They share
  the data folder and the server token, and the server routes each run by the entry's ground.
- **Secrets** live in 0600 files: `api.json` (the local key), `mqtt.json` (the broker
  password), `server.json` (the agent token). The account password is never stored.
- **HomeKit identity.** Never change the UUID seeds or the `homekit/` storage layout; users
  would lose their pairings, rooms and scenes.
- **Locks.** Don't hold a `std::sync::Mutex` guard across `.await`; scope the guard first. The
  server's agent session once did.
- **Phone link.** Never change the crypto (labels, AAD, HKDF info) without regenerating the
  vectors and updating `PhoneCrypto.swift`; paired phones would stop working. Keys live in
  `phones.json` (Mac) and the Keychain (`computer-key-<macId>`, iPhone).
- **Parameters** are appended to the command line, unless it mentions `CMDTRIGGER_PARAMS`, which
  then carries them alone. Test commands that check their output end with `; true`.
- **Platform code.** `cfg(windows)` and Linux-only code is checked only by
  `npm run check:windows` / `check:linux`, since a macOS build never sees it. A helper used on
  only some platforms needs a matching `cfg`, or clippy fails on the others with dead code.

## Design

The UI follows `DESIGN.md` (adopted 2026-11-04, replacing the old yellow system):
- **Colours:** one canvas `#101010`, with `canvas-soft` `#1a1a1a` for inputs, code and dialogs.
  Cards are the canvas plus a 1 px hairline `#3d3a39`, with no fills and no shadows. The one
  accent is electric green `#00d992`, for primary actions, live status and the `>_` mark only.
  Red `#f6465d` is only for "failed".
- **Type:** Inter (`@fontsource`) with SF Mono (JetBrains Mono as the fallback off Apple).
  Section labels are tracked uppercase eyebrows.
- **Shapes:** buttons 6 px, cards 8 px; pills only for tags and status.
- **Dialogs and sheets** are dark too (the owner's preference).
- **Tokens:** `src/styles.css` `:root`, `crates/cmdtrigger-server/dashboard/app.css`, and
  `ios/CmdTrigger/Theme.swift` (`.hairline()` is the card chrome).
- **Icons:** the app icons come from `assets/icon.svg` and `assets/icon-ios.svg`. After
  changing them, run `rsvg-convert` + `npx tauri icon assets/icon.png -o src-tauri/icons`, then
  delete the generated `android/` and `ios/` folders and keep `tray*.png`.
- **Checking it:** the UI is checked in headless Chrome with a mocked `__TAURI_INTERNALS__`, and
  the iPhone app through `ios/ui-test.sh` screenshots.
