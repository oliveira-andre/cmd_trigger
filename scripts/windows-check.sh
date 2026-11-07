#!/usr/bin/env bash
# Cross-checks the Windows build from any machine with Docker: `npm run check:windows`.
# Compiles and lints every crate for x86_64-pc-windows-gnu (so the cfg(windows) code is
# type-checked), builds the CLI .exe, and runs the core's unit tests under Wine 10 (Debian
# trixie, x86-64; bookworm's Wine 8 lacks bcryptprimitives.dll, which Rust's std needs).
set -euo pipefail

if [[ "${CMDTRIGGER_IN_CONTAINER:-}" != 1 ]]; then
  root="$(cd "$(dirname "$0")/.." && pwd)"
  mkdir -p "$root/target/windows-check"
  [[ -f "$root/dist/index.html" ]] || (cd "$root" && npm run build > /dev/null)
  docker run --rm -e CMDTRIGGER_IN_CONTAINER=1 \
    -v "$root":/src:ro \
    -v "$root/target/windows-check":/out \
    -v cmdtrigger-linux-cargo:/usr/local/cargo/registry \
    -v cmdtrigger-windows-target:/work/target \
    rust:1-bookworm /src/scripts/windows-check.sh
  # The .exe files are x86-64; Wine needs an x86-64 container (emulated on Apple Silicon).
  echo; echo "== core unit tests and cmdtrigger.exe under Wine (x86-64)"
  exec docker run --rm --platform linux/amd64 -v "$root/target/windows-check":/out debian:trixie-slim bash -c '
    set -eo pipefail
    apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends wine64 > /dev/null 2>&1
    export WINEDEBUG=-all WINEPREFIX=/tmp/wineprefix XDG_RUNTIME_DIR=/tmp/xdg; mkdir -p -m 700 $XDG_RUNTIME_DIR
    wine=/usr/lib/wine/wine64; $wine --version
    $wine /out/core-tests.exe 2>/dev/null | tee /out/test.log | grep -E "test result|FAILED|panicked|runs_through_cmd_exe"
    $wine /out/cmdtrigger.exe --version 2>/dev/null'
fi

export DEBIAN_FRONTEND=noninteractive
step() { echo; echo "== $*"; }
target=x86_64-pc-windows-gnu

step "toolchain"
dpkg --add-architecture amd64 2>/dev/null || true
apt-get update -qq
apt-get install -y -qq --no-install-recommends gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64 cmake nasm clang > /dev/null
rustup target add $target > /dev/null 2>&1
rustup component add clippy > /dev/null 2>&1
rustc --version

mkdir -p /work
(cd /src && tar --exclude=./node_modules --exclude=./target --exclude=./src-tauri/gen --exclude=./.dev-data -cf - .) | tar -xf - -C /work
cd /work

step "clippy (windows)"
cargo clippy --workspace --all-targets --target $target -- -D warnings 2>&1 | tee /out/clippy.log | grep -E '^(warning|error)|Finished' | tail -20

step "build cmdtrigger.exe"
cargo build -p cmdtrigger-cli --target $target 2>&1 | tail -1
ls -la target/$target/debug/cmdtrigger.exe
cp target/$target/debug/cmdtrigger.exe /out/

step "build the core unit tests"
cargo test -p cmdtrigger-core --lib --target $target --no-run 2>&1 | tee /out/test-build.log | tail -2
exe=$(grep -o 'target/[^ )]*cmdtrigger_core-[0-9a-f]*\.exe' /out/test-build.log | tail -1)
cp "$exe" /out/core-tests.exe && echo "copied $exe"
