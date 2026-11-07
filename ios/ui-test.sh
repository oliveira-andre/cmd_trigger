#!/usr/bin/env bash
# End-to-end test of the iPhone app in the simulator against a real CmdTrigger on this Mac:
# the debug CmdTrigger.app (phone link), a background agent, and a cmdtrigger-server, all on
# scratch data and non-default ports, so a CmdTrigger you use day to day isn't touched.
#
#   ios/ui-test.sh            build what's needed, run, leave screenshots in ios/build/shots
#
# Needs: `npx tauri build --debug --bundles app` and `cargo build -p cmdtrigger-cli -p
# cmdtrigger-server` done (the script runs them), Xcode with an iPhone simulator.
set -euo pipefail
cd "$(dirname "$0")/.."
REPO=$(pwd)
export PATH="$HOME/.cargo/bin:$PATH"
SIM=${SIM:-"iPhone 17"}
SHOTS=$REPO/ios/build/shots
S=$(mktemp -d "${TMPDIR:-/tmp}/cmdtrigger-ui.XXXXXX")
D=$S/data
M=$S/marker.txt
APP=$REPO/target/debug/bundle/macos/CmdTrigger.app
AGENT="$APP/Contents/MacOS/cmdtrigger-agent"
CLI=$REPO/target/debug/cmdtrigger
PASSWORD="correct horse battery"
pids=()

cleanup() {
  pkill -f "$AGENT" 2>/dev/null || true
  for p in ${pids[@]+"${pids[@]}"}; do kill "$p" 2>/dev/null || true; done
}
trap cleanup EXIT

step() { echo; echo "== $*"; }

step "build"
cargo build -q -p cmdtrigger-cli -p cmdtrigger-server
npx tauri build --debug --bundles app 2>&1 | grep -E "Finished|error" | tail -1
xcodebuild -project ios/CmdTrigger.xcodeproj -scheme CmdTrigger -destination "platform=iOS Simulator,name=$SIM" \
  -derivedDataPath ios/build/dd build-for-testing 2>&1 | grep -E "error:|BUILD|TEST BUILD" | tail -2

step "scratch computer \"Live Mac\" in $S"
mkdir -p "$D" "$SHOTS"
rm -f "$SHOTS"/*.png "$SHOTS/wake-tapped"
: > "$M"
cat > "$D/commands.json" <<EOF
[
 {"name": "Marker", "command": "echo \"\$CMDTRIGGER_SOURCE:\$CMDTRIGGER_PARAMS\" >> $M; echo marked; true", "allowParams": true, "icon": "📝", "description": "Writes a line to a file"},
 {"name": "Fails", "command": "echo boom; exit 5", "icon": "💥"},
 {"name": "Lights", "command": "echo lights-on >> $M; true", "offCommand": "echo lights-off >> $M; true", "allowParams": true, "icon": "💡"},
 {"name": "Boot", "command": "echo \"bg:\$CMDTRIGGER_SOURCE\" >> $M; true", "ground": "background", "icon": "🖥️"}
]
EOF
cat > "$D/automations.json" <<EOF
[
 {"id": "morning01", "name": "Morning", "icon": "☀️", "steps": [{"type": "command", "command": "Marker", "params": "morning"}]},
 {"id": "wol01", "name": "Wake test", "steps": [{"type": "wakeOnLan", "mac": "02:00:00:00:00:01", "broadcast": "127.0.0.1"}]}
]
EOF
echo '{"computerName": "Live Mac", "language": "en"}' > "$D/settings.json"
echo '{"enabled": true, "port": 7391, "allowLan": false, "key": "ct_uitest00000000000000000000000000000000000000000000"}' > "$D/api.json"
echo '{"enabled": true, "port": 7380}' > "$D/phones.json"

CMDTRIGGER_DB=$S/server.db CMDTRIGGER_ADDR=127.0.0.1:8093 ./target/debug/cmdtrigger-server > "$S/server.log" 2>&1 &
pids+=($!)
sleep 1
printf '%s\n' "$PASSWORD" | CMDTRIGGER_DATA_DIR=$D "$CLI" connect http://127.0.0.1:8093 --email phone@example.com --create | tail -1
start_app() { open -n -g --env CMDTRIGGER_DATA_DIR="$D" "$APP"; }
start_app
CMDTRIGGER_DATA_DIR=$D "$CLI" --run "$D" > "$S/background.log" 2>&1 &
pids+=($!)
for _ in $(seq 40); do curl -sf localhost:7380/phone/v1/hello > /dev/null && break; sleep 0.25; done
curl -s localhost:7380/phone/v1/hello; echo

run_test() {
  local name=$1 code=${2:-}
  TEST_RUNNER_CT_CODE=$code TEST_RUNNER_CT_NAME="Live Mac" TEST_RUNNER_CT_SHOTS=$SHOTS \
  TEST_RUNNER_CT_SERVER=http://127.0.0.1:8093 TEST_RUNNER_CT_EMAIL=phone@example.com TEST_RUNNER_CT_PASSWORD=$PASSWORD \
  xcodebuild -project ios/CmdTrigger.xcodeproj -scheme CmdTrigger -destination "platform=iOS Simulator,name=$SIM" \
    -derivedDataPath ios/build/dd -only-testing:"CmdTriggerUITests/CmdTriggerUITests/$name" test-without-building 2>&1 |
    tee "$S/$name.log" | grep -E "error:|Test Case .* (passed|failed)|TEST (SUCCEEDED|FAILED)" || true
}

step "1: pair over Wi-Fi (Bonjour + code), run, server sign-in"
CODE=$(CMDTRIGGER_DATA_DIR=$D "$CLI" pair --json | python3 -c 'import json,sys; print(json.load(sys.stdin)["code"])')
echo "pairing code $CODE"
run_test test1PairAndRun "$CODE"

step "2: the app quits; the phone runs the background command through the server"
pkill -f "$AGENT" || true
sleep 2
run_test test2ThroughServer

step "3: wake: the phone keeps knocking; the app comes back 6 s after the tap"
rm -f "$SHOTS/wake-tapped"
(for _ in $(seq 240); do [ -f "$SHOTS/wake-tapped" ] && break; sleep 0.5; done; sleep 6; start_app) &
run_test test3Wake

step "on the Mac"
echo "paired phones: $(python3 -c "import json; print([p['name'] for p in json.load(open('$D/phones.json'))['phones']])")"
echo "marker file:"; sed 's/^/  /' "$M"
grep -q "^phone:from-iphone$" "$M" && echo "PASS  params from the iPhone reached the command" || echo "FAIL  no phone:from-iphone"
grep -q "^lights-off$" "$M" && echo "PASS  on/off" || echo "FAIL  lights-off"
grep -q "^bg:server/api$" "$M" && echo "PASS  background command through the server" || echo "FAIL  bg:server/api"
echo "screenshots: $SHOTS"
