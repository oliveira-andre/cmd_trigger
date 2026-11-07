#!/usr/bin/env bash
# Builds, checks, tests and smoke-runs CmdTrigger on Debian Linux inside Docker, from any
# machine with Docker: `npm run check:linux`. Logs land in target/linux-check/.
set -euo pipefail

if [[ "${CMDTRIGGER_IN_CONTAINER:-}" != 1 ]]; then
  root="$(cd "$(dirname "$0")/.." && pwd)"
  mkdir -p "$root/target/linux-check"
  exec docker run --rm -e CMDTRIGGER_IN_CONTAINER=1 \
    -v "$root":/src:ro \
    -v "$root/target/linux-check":/out \
    -v cmdtrigger-linux-cargo:/usr/local/cargo/registry \
    -v cmdtrigger-linux-target:/work/target \
    rust:1-bookworm /src/scripts/linux-check.sh
fi

export DEBIAN_FRONTEND=noninteractive
step() { echo; echo "== $*"; }

step "system libraries"
apt-get update -qq
apt-get install -y -qq --no-install-recommends \
  libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev patchelf file \
  curl xz-utils xvfb xauth dbus-x11 > /dev/null

step "node"
case "$(uname -m)" in aarch64) arch=arm64 ;; x86_64) arch=x64 ;; esac
curl -fsSL "https://nodejs.org/dist/v24.19.0/node-v24.19.0-linux-${arch}.tar.xz" | tar -xJ -C /opt
export PATH="/opt/node-v24.19.0-linux-${arch}/bin:$PATH"
node --version; rustc --version; uname -m

mkdir -p /work
(cd /src && tar --exclude=./node_modules --exclude=./target --exclude=./dist --exclude=./src-tauri/gen --exclude=./.dev-data -cf - .) | tar -xf - -C /work
cd /work

step "npm ci"; npm ci --no-audit --no-fund --loglevel=error
step "typecheck"; npx tsc --noEmit && echo ok
rustup component add clippy rustfmt > /dev/null 2>&1
step "fmt"; cargo fmt --all --check && echo ok
step "clippy"
if cargo clippy --workspace --all-targets -- -D warnings > /out/clippy.log 2>&1; then echo ok
else grep -E -A15 '^(error|warning)' /out/clippy.log | head -80; exit 1; fi
step "test"; cargo test --workspace 2>&1 | tee /out/test.log | grep -E 'test result|FAILED|panicked|^error'
step "bundle"; npx tauri build --debug --bundles deb 2>&1 | grep -E 'Finished|Bundling|error' ; ls -la target/debug/bundle/deb/

step "smoke run: server + tray app (foreground) + --run (background) on one computer"
cargo build -q -p cmdtrigger-cli -p cmdtrigger-server
set +e
CMDTRIGGER_DB=/tmp/server.db CMDTRIGGER_ADDR=127.0.0.1:8090 ./target/debug/cmdtrigger-server > /out/server.log 2>&1 &
for _ in $(seq 30); do curl -sf localhost:8090/api/v1/health > /dev/null && break; sleep 0.5; done
json() { python3 -c "import json,sys; print(json.load(sys.stdin)$1)"; }
SESSION=$(curl -s -XPOST localhost:8090/api/v1/signup -H 'content-type: application/json' -d '{"email":"me@example.com","password":"correct horse"}' | json "['token']")
TOKEN=$(curl -s -XPOST localhost:8090/api/v1/tokens -H "authorization: Bearer $SESSION" -H 'content-type: application/json' -d '{"name":"smoke"}' | json "['token']")
D=/tmp/data; mkdir -p $D
echo '{"computerName": "Smoke"}' > $D/settings.json
cat > $D/commands.json <<JSON
[{"name": "Desk", "command": "printf fg > /tmp/fg.txt", "ground": "foreground"},
 {"name": "Boot", "command": "printf bg > /tmp/bg.txt", "ground": "background"}]
JSON
cat > $D/automations.json <<JSON
[{"id": "hooked", "name": "Hooked", "triggers": [{"type": "webhook", "hook": "smokehook123"}], "steps": [{"type": "shell", "line": "printf hook > /tmp/hook.txt"}]}]
JSON
printf 'correct horse\n' | CMDTRIGGER_DATA_DIR=$D ./target/debug/cmdtrigger connect http://127.0.0.1:8090 --email me@example.com
export WEBKIT_DISABLE_COMPOSITING_MODE=1 WEBKIT_DISABLE_DMABUF_RENDERER=1
CMDTRIGGER_DATA_DIR=$D dbus-run-session -- xvfb-run -a ./target/debug/cmdtrigger-agent > /out/app.log 2>&1 &
./target/debug/cmdtrigger --run $D > /out/service.log 2>&1 &
sleep 10
for target in Desk Boot; do
  curl -s -XPOST localhost:8090/api/v1/run -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' \
    -d "{\"computer\":\"Smoke\",\"target\":\"$target\",\"wait\":10}" | json "['status']" | sed "s/^/$target via server: /"
done
curl -s -XPOST localhost:7171/hooks/smokehook123 | json "['status']" | sed 's/^/webhook: /'
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | CMDTRIGGER_DATA_DIR=$D ./target/debug/cmdtrigger mcp | json "['result']['tools'].__len__()" | sed 's/^/mcp tools: /'
echo "foreground ran: $(cat /tmp/fg.txt 2>/dev/null || echo NO)"
echo "background ran: $(cat /tmp/bg.txt 2>/dev/null || echo NO)"
echo "webhook ran: $(cat /tmp/hook.txt 2>/dev/null || echo NO)"
curl -s localhost:8090/api/v1/computers -H "authorization: Bearer $TOKEN" | json "[0]['online']" | sed 's/^/agents online: /'
pgrep -f cmdtrigger-agent > /dev/null && echo "tray app still running" || echo "tray app exited"
