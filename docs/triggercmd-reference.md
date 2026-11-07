# TRIGGERcmd Agent 1.0.63: reverse-engineering reference

What CmdTrigger was built from. Target: `/Applications/TRIGGERcmdAgent.app` (bundle id
`com.electron.triggercmdagent`, version 1.0.63, Electron 38, `LSUIElement`). The app ships its
JavaScript unpacked in `Contents/Resources/app`, so everything below was read from the shipped
source and cross-checked with REA's static analysis. None of it was observed at runtime against
triggercmd.com.

> **Historical.** CmdTrigger 0.1 reproduced this agent, including its triggercmd.com sign-in,
> sync and Socket.IO protocol. From 0.2 that code is gone: CmdTrigger has no TRIGGERcmd account
> and never contacts triggercmd.com. Remote runs go through your own `cmdtrigger-server`. What
> carried over:
> - the `commands.json` field names, which `model::Command::normalize` reads;
> - a one-time import from `~/.TRIGGERcmdData`;
> - the HomeKit bridge's hap-nodejs storage format;
> - the platform default commands.
>
> Read the rest as the record of what the original does, not as what CmdTrigger does.

## REA evidence ledger

| Evidence | What | Coverage |
| --- | --- | --- |
| `ev_3e174a3b…f7` | `analyze_javascript_application` on the whole `app/` folder (5,536 JS files, incl. `node_modules`) | partial: the result budget went to `node_modules`, and semantic analysis of `src/*.js` never started |
| `ev_f80963b1…8d` | the same analysis without `node_modules` (38 files) | partial: the bundled React editor (`app/js`, `process/js`) used the budget |
| `ev_a54fbe5f…0f` | `src/*.js` + `package.json` only (18 files) | application graph **complete**, semantic families partial |

These are session-retained REA records. The exported bundles (250–310 MB) lived in that
session's scratchpad. Re-run `analyze_javascript_application` with `detail: "summary"` on a
`src`-only copy to get them back; on the full tree, the summary itself overflows MCP.

Facts taken from `ev_a54fbe5f…0f`:
- **IPC (13 literal channels, 7 paired):** `did-submit-form`, `homeAssistantSave`, `homeKitSave`,
  `wakeOnLanSave`, `load-ha-config`, `load-homekit-config`, `load-wol-config`; renderer-only:
  `did-finish-load`, `processing-did-succeed`, `processing-did-fail`; GUI editor:
  `exampleAdded`, `openexampleWindow`, `closeexampleWindow`.
- **9 BrowserWindows,** all `nodeIntegration: true, contextIsolation: false,
  enableRemoteModule: true`: sign-in, editor, example list, text editor, HA, HomeKit, WOL,
  computer list. `editormain.js` also has `webSecurity: false`.
- **Timers (exact lines):**

  | where | what |
  | --- | --- |
  | `agent.js:308` | 10 s offline retry |
  | `agent.js:507` | 5 min sync watchdog |
  | `agent.js:688` | 10 ms spacing between adds |
  | `agent.js:913` | 1 s background debounce |
  | `ha.js:171` | 100 ms before auth |
  | `ha.js:314` | reconnect backoff |
  | `homekitbridge.js:169` | 700 ms kill |
  | `homekitbridge.js:176` | 1 s restart |
  | `homekitprocess.js:168` | 800 ms "on" delay |
  | `homekitprocess.js:280` | 500 ms exit |

- **Environment and argv reads:** `HOME`/`USERPROFILE`/`HOMEPATH` for the data folder;
  `--daemoninstall`, `--console` (agent.js), `--run <dir>` (daemon.js), `--install`/`--remove`
  (svcmgr, winpath), and the Squirrel events (main.js:1351).

## Process layout

- `src/main.js`: the Electron main process. It handles the tray, windows, IPC, posix-argv-parser
  flags (`-t/--trigger/--command`, `-c/--computer`, `-h`, the Squirrel flags), the single-instance
  lock, auto-launch, i18next and the background service install.
- `src/agent.js`: the agent. Its exports are `background`, `foreground`, `getToken`,
  `tokenLogin`, `createComputer`, `computerExists`, `triggerCmd`, `initFiles`,
  `cmdFileUpdated`, `fetchexamples`, `restart/startHomeAssistant`, `restart/startHomeKit`,
  `updateWakeOnLan` and `getMacAddresses`. It is also runnable directly with `--console` or
  `--daemoninstall`.
- `src/ha.js`: the Home Assistant websocket client (`ws`).
- `src/homekitbridge.js`: spawns `homekitprocess.js` with `ELECTRON_RUN_AS_NODE=1`.
  - That child runs `hap-nodejs` 1.2.0, with a chacha20-poly1305 shim for BoringSSL.
  - Control is JSON lines on stdin: `{"type":"stop"}` and `{"type":"reload"}`.
- `src/daemon.js` (`--run <dir>`, Linux systemd), `src/service.js` and `src/svcmgr.js`
  (Windows, `node-windows`), `src/triggercmd.js` (remote trigger CLI).
- `app/js/render.js`: the React GUI editor, browserify-bundled. Components: `AptList`,
  `AddAppointment`, `EditAppointment`, `HeaderNav`, `Toolbar`, `EmojiPicker`. `examples.js` is
  the example browser.

## Data folder `~/.TRIGGERcmdData`

| file | contents |
| --- | --- |
| `token.tkn` | the bearer token, as typed (no newline) |
| `computerid.cfg` / `userid.cfg` | IDs from the API |
| `computername.cfg` | the account's name for this computer, cached by the HA module |
| `commands.json` | the command list; `.backup` is the last version that parsed |
| `onlineexamples.json` | `/api/example/list` records, 1-space indent |
| `home_assistant_config.json` | `HA_URL`, `HA_TOKEN`, `HA_ENABLED` |
| `homekit_config.json` | `HK_ENABLED`, `HK_BRIDGE_NAME`, `HK_PIN` (`XXX-XX-XXX`), `HK_USERNAME` (MAC-style), `HK_DEBUG_LOGGING`; `HK_PORT` is deleted on load |
| `wake_on_lan_config.json` | `WOL_ENABLED` |
| `language.cfg` | `en`, `es`, `pt`, `de`, `fr`, `jp` or `it` |
| `debug.log` | truncated at each GUI start |
| `sendresult.sh` / `SendResult.bat` | result helper |
| `cache/` | hap-nodejs storage (node-persist 0.0.12: file name = key, content = JSON): `AccessoryInfo.<USER>.json`, `IdentifierCache.<USER>.json` |

**Command fields:** `trigger`, `command`, `offCommand`, `ground` (`foreground` or `background`),
`voice`, `voiceReply`, `allowParams`, `quoteParams`, `mcpToolDescription` and `icon`.
`allowParams` and `quoteParams` are usually the strings `"true"`/`"false"` (from the GUI's
selects), sometimes booleans.

## REST API (base `https://www.triggercmd.com`)

Every call sends `Authorization: Bearer <token>` and `User-Agent: TRIGGERcmd/0.1.0`, with
`application/x-www-form-urlencoded` bodies (the `request` library with `qs`: booleans become
`"true"`/`"false"`, `undefined` fields are left out). On Linux the original pins the GoDaddy G2
CA bundle.

| call | body / query | reply |
| --- | --- | --- |
| `POST /api/auth/authenticate` | `email, password` | `{token, user:{id}}` (unused by the UI) |
| `GET /api/command/list` | - | used to validate a pasted token (200 = valid) |
| `GET /api/command/list?computer_id=X&includeDuplicates=1` | - | `{records:[{name, voice, voiceReply, allowParams, mcpToolDescription, icon, …}]}` |
| `GET /api/computer/list?computer_id=X` | - | `{records:[{id, name}]}`; empty means deleted |
| `GET /api/computer/list?computer_id=X&token=T` | token in the query, no header (ha.js) | computer name |
| `POST /api/computer/save` | `name` | `{data:{id}}`; **403** means "must subscribe" |
| `POST /api/command/save` | `name, computer, voice, voiceReply, allowParams, mcpToolDescription, icon` | - |
| `POST /api/command/delete2` | `name, computer` (deletes every copy with that name) | - |
| `POST /api/run/save` | `status, computer, command` (the command's id) | `{data:{id}}` |
| `POST /api/run/triggerSave` | `computername, triggername` | `{message}` |
| `GET /api/example/list` | - | `{records:[{name, command, ground, voice, voiceReply, allowParams, mcpToolDescription, icon, os}]}` |
| `POST /api/computer/updateWakeOnLan` | `computer_id, enableWakeOnLan, macAddresses` (a JSON-encoded array) | - |
| `POST /api/command/result` | multipart `computer_id, command_id, result` (sendresult.sh) | - |

## Live socket

The client stack is `sails.io.js` 1.2.1, `socket.io-client` 2.5.0, `engine.io-client` 3.5.6
(Engine.IO **protocol 3**) and `socket.io-parser` 3.3.6, with the websocket transport only.

- **URL:** `wss://www.triggercmd.com/socket.io/?__sails_io_sdk_version=1.2.1&__sails_io_sdk_platform=node&__sails_io_sdk_language=javascript&EIO=3&transport=websocket`.
- **Headers:** none at connect time; `io.sails.headers` are per request.
- **Handshake:** the server opens with `0{"sid","pingInterval","pingTimeout"}`, then `40`. In
  protocol 3 the **client** sends `2` every `pingInterval` and expects `3`.
- **Room join**, on every connect:
  `42<n>["get",{"method":"get","headers":{"Authorization":"Bearer T"},"data":{"Authorization":"Bearer T"},"url":"/api/computer/subscribeToFunRoom?roomName=<computerId>"}]`.
  The reply is `43<n>[{"body":…,"headers":…,"statusCode":…}]`.
- **Triggers:** `42["message",{"trigger","id","params","sender"}]`. `sender` is `"Home Assistant"`
  for HA-originated runs.
- **Reconnect:** socket.io's backoff, 1 s × 2ⁿ ± 50 %, at most 5 s, with no attempt limit. Each
  reconnect also forces the HA websocket to reconnect.

## Behaviour details

- **Launch:**
  - With no token, it shows the sign-in window (700×470) and no tray.
  - With a token, the tray says "Waiting for internet." while `computerExists` runs. It retries
    every 10 s only on `getaddrinfo` errors; any other failure leaves it waiting forever.
  - If the computer exists: `foreground()` and the full tray. If not: the sign-in window.
- **Sign-in:** `tokenLogin` (GET command list), then `createComputer(name || os.hostname())`.
  A 403 shows `subscribefirst.html`.
- **Sync:** see `crates/cmdtrigger-core/src/sync.rs`, a port of `planSync`, `cmdMatches`,
  `updateCmds`, `startSync`/`finishSync` and `watchForCmdUpdates`.
  - `cmdMatches` compares `name`, `voice`, `voiceReply` and `mcpToolDescription` with loose `==`,
    `Boolean(online.allowParams)` against local `allowParams ∈ {1,"1",true,"true"}`, and
    `icon || ''`.
- **Trigger handling** (agent.js 783–841):
  - The command is matched by exact trigger, then filtered by `ground`.
  - If HA is connected and `sender == "Home Assistant"`, the trigger is ignored.
  - Otherwise `TCMD_COMPUTER_ID`/`TCMD_COMMAND_ID` go into `process.env` (permanently), and it
    reports "Command ran".
  - It then runs `child_process.exec`, and on error reports "Command exited with error code
    <error.code>".
- **Home Assistant:**
  - The prefix is `switch.` + computer name lowercased with `[-\s]` → `_`, + `_`.
  - The trigger is matched as `trigger.toLowerCase().replace(/ /g,'_')`.
  - The parameter is `on` for `turn_on` and `off` for `turn_off`. Quoting is never applied.
  - Reconnect delay is `min(10 ms × 2ⁿ, 30 s)`.
- **HomeKit (hap-nodejs):**
  - **IDs:**
    - The bridge UUID is `uuid.generate("triggercmd-homekit-bridge:"+username)`, and each light's
      is `uuid.generate("triggercmd-homekit:"+computerName+" "+trigger)`.
    - `uuid.generate` is SHA-1 laid out as `xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx`.
    - aid and iid come from the IdentifierCache (aid from 2; iid from 2 per accessory;
      AccessoryInformation is iid 1; the bridge's non-information services get +2,000,000,000).
  - **Lights:** each has Lightbulb service subtype `lightbulb` with Name, On and Brightness.
  - **Bridge:** port 47129, category BRIDGE (2).
  - **SRP:** `fast-srp-hap` with the 3072-bit group and SHA-512.
    - `K = H(PAD(S))`.
    - `M1 = H(H(N) xor H(g) | H(I) | s | A | B | K)`, with `N` and `g` unpadded in the first term.
    - Vectors are in `crates/cmdtrigger-core/src/homekit/srp_vectors.json`.
  - **mDNS TXT:** `c# ff id md pv=1.1 s#=1 sf ci=2 sh`, where `sh` is base64 of the first 4 bytes
    of SHA-512(setupID + username).

## Observed live (2026-11-03, no account)

`cargo run -p cmdtrigger-core --example probe` against https://www.triggercmd.com:
- `GET /api/computer/list` with a bad token returns `401 {"err":"Invalid Token!"}`.
- The socket URL above upgrades (101). The server sends
  `0{"sid":…,"upgrades":[],"pingInterval":25000,"pingTimeout":60000}` then `40`.
- The room-join request is answered as
  `431[{"body":{"err":"Invalid Token!"},"headers":{},"statusCode":401}]`.
- A client ping `2` gets `3`.

The mock server returns these same error bodies.

## What was not observed

- No authenticated traffic: there is no account on this machine. So the server's command-list
  field types, `includeDuplicates` and a real trigger's `message` payload come from what the
  client expects of them, not from observation.
- REA's semantic families stayed partial (call-flow, data-flow, object-flow unknowns).
  Behaviour claims rest on reading the source, with REA's IPC, window, timer and config facts
  as cross-checks.
