#!/usr/bin/env bash
# Vercel build of the website with the real engine (docs/HOSTING.md).
#
# Vercel runs this from web/ (the project's root directory) after `npm ci`. It:
#   1. installs clang and llvm (dnf), if missing. secp256k1-sys, a dependency of the
#      engine, compiles C code for wasm32;
#   2. installs Rust 1.97.0 with the wasm32-unknown-unknown target (rustup, minimal
#      profile), if missing;
#   3. downloads the wasm-bindgen 0.2.129 release binary and checks its SHA-256;
#   4. builds the engine into web/public/engine/ (crates/zecret-wasm/build.sh);
#   5. builds the site with VITE_ZECRET_CONFIG=testnet unless the variable is set. A
#      mainnet build (VITE_ZECRET_CONFIG=mainnet, a separate Vercel project, docs/MAINNET.md)
#      stops before any work unless VITE_ZECRET_ALLOW_MAINNET=1, VITE_MAINNET_ESCROW and
#      VITE_MAINNET_ESCROW_CODE_HASH are set and web/public/config/mainnet-directory.json exists;
#   6. stops if the value of ONECLICK_JWT (the 1Click partner key) is in the built site.
#
# It also runs on any Linux x86_64 machine with dnf or with these tools already present.
set -euo pipefail

RUST_TOOLCHAIN=1.97.0
WASM_BINDGEN_VERSION=0.2.129
WASM_BINDGEN_SHA256=82d12bb940e2d4e72e0d5605387fc1b8ca179044e012b620f0ce4e7440e8320e

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TOOLS="${ZECRET_BUILD_TOOLS:-$HOME/.zecret-build-tools}"
mkdir -p "$TOOLS/bin"
export PATH="$TOOLS/bin:$HOME/.cargo/bin:$PATH"
t0=$(date +%s)
step() { echo "[vercel-build +$(( $(date +%s) - t0 ))s] $*"; }

if [ "${VITE_ZECRET_CONFIG:-}" = mainnet ]; then
  [ "${VITE_ZECRET_ALLOW_MAINNET:-}" = 1 ] || { echo "mainnet build: set VITE_ZECRET_ALLOW_MAINNET=1" >&2; exit 1; }
  [ -n "${VITE_MAINNET_ESCROW:-}" ] && [ -n "${VITE_MAINNET_ESCROW_CODE_HASH:-}" ] || { echo "mainnet build: set VITE_MAINNET_ESCROW and VITE_MAINNET_ESCROW_CODE_HASH" >&2; exit 1; }
  [ -s "$ROOT/web/public/config/mainnet-directory.json" ] || { echo "mainnet build: web/public/config/mainnet-directory.json is missing (docs/MAINNET.md, the directory entry)" >&2; exit 1; }
  step "mainnet build: escrow $VITE_MAINNET_ESCROW, code hash $VITE_MAINNET_ESCROW_CODE_HASH"
fi

# 1. C compiler for wasm32.
if ! command -v clang >/dev/null || ! command -v llvm-ar >/dev/null; then
  step "installing clang and llvm with dnf"
  if [ "$(id -u)" = 0 ]; then dnf install -y -q clang llvm; else sudo dnf install -y -q clang llvm; fi
fi
step "$(clang --version | head -1)"

# 2. Rust.
if ! command -v rustup >/dev/null; then
  step "installing rustup and Rust $RUST_TOOLCHAIN"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --no-modify-path --profile minimal --default-toolchain "$RUST_TOOLCHAIN"
fi
rustup toolchain install "$RUST_TOOLCHAIN" --profile minimal --target wasm32-unknown-unknown
export RUSTUP_TOOLCHAIN="$RUST_TOOLCHAIN"
step "$(rustc --version)"

# 3. wasm-bindgen, the release binary, checked against the pinned hash.
if [ "$(wasm-bindgen --version 2>/dev/null || true)" != "wasm-bindgen $WASM_BINDGEN_VERSION" ]; then
  step "downloading wasm-bindgen $WASM_BINDGEN_VERSION"
  name="wasm-bindgen-$WASM_BINDGEN_VERSION-x86_64-unknown-linux-musl"
  curl -sSLf -o "$TOOLS/wb.tgz" \
    "https://github.com/wasm-bindgen/wasm-bindgen/releases/download/$WASM_BINDGEN_VERSION/$name.tar.gz"
  echo "$WASM_BINDGEN_SHA256  $TOOLS/wb.tgz" | sha256sum -c -
  tar -xzf "$TOOLS/wb.tgz" -C "$TOOLS"
  install -m 0755 "$TOOLS/$name/wasm-bindgen" "$TOOLS/bin/wasm-bindgen"
fi
step "$(wasm-bindgen --version)"

# 4. The engine. CC and AR for the C parts of the wasm32 build.
step "building the engine (crates/zecret-wasm)"
export CC_wasm32_unknown_unknown=clang AR_wasm32_unknown_unknown=llvm-ar
"$ROOT/crates/zecret-wasm/build.sh"
step "engine built"

# 5. The site.
export VITE_ZECRET_CONFIG="${VITE_ZECRET_CONFIG:-testnet}"
step "building the site with VITE_ZECRET_CONFIG=$VITE_ZECRET_CONFIG"
cd "$ROOT/web"
npm run build
# The 1Click partner key (ONECLICK_JWT, docs/ONECLICK_PROXY.md) is for the function in
# web/api/ only. Vite puts only VITE_* variables into the bundle; this check stops the
# deployment if the key's value is in the built site anyway. It never prints the key.
# The value is trimmed as the function trims it; a JWT's signature part is checked on its
# own too, so a key cut into pieces or with its header changed is still found.
ONECLICK_KEY="$(printf '%s' "${ONECLICK_JWT:-}" | tr -d '[:space:]')"
if [ -n "$ONECLICK_KEY" ]; then
  ONECLICK_SIG="${ONECLICK_KEY##*.}"
  [ "${#ONECLICK_SIG}" -ge 16 ] || ONECLICK_SIG="$ONECLICK_KEY"
  if grep -rqF -e "$ONECLICK_KEY" -e "$ONECLICK_SIG" dist; then
    echo "the 1Click partner key is in web/dist: stop (docs/ONECLICK_PROXY.md)" >&2
    exit 1
  fi
fi
step "done"
