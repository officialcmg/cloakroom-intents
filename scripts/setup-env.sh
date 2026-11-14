#!/usr/bin/env bash
# Prepare a machine for building and testing Cloakroom Intents.
# Safe to run more than once. Puts tools in .tools/ (not committed).
set -euo pipefail

# 0. Supported host and required tools. The NEAR sandbox below is a Linux x86-64 binary.
if [ "$(uname -s)" != "Linux" ] || [ "$(uname -m)" != "x86_64" ]; then
  echo "setup-env.sh needs Linux x86-64 (this is $(uname -s) $(uname -m)). The hosted site (https://cloakroom.chrismg.dev) and the web/ unit tests (cd web && npm ci && npm test) work on any OS." >&2
  exit 1
fi
# This script cannot run without rustup, curl and python3. The tests need the others.
missing="" later=""
for tool in rustup curl python3; do command -v "$tool" >/dev/null || missing="$missing $tool"; done
for tool in docker node gcc; do command -v "$tool" >/dev/null || later="$later $tool"; done
if [ -n "$missing" ]; then
  echo "setup-env.sh: missing:$missing${later:+ (also missing:$later)}. Install them and run it again." >&2
  exit 1
fi
[ -z "$later" ] || echo "warning: missing:$later. Docker runs the local Zcash node, node runs the website and browser tests, gcc builds the open-file shim." >&2

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TOOLS="$ROOT/.tools"
mkdir -p "$TOOLS/wasm"

# 1. Rust targets.
rustup target add wasm32-unknown-unknown wasm32-wasip1 >/dev/null

# 2. NEAR sandbox. It must match mainnet (2.14.1 on 2026-11-13): older sandboxes
#    reject current contract builds and the live intents.near code.
SANDBOX_VERSION="${SANDBOX_VERSION:-2.14.1}"
if [ ! -x "$TOOLS/near-sandbox" ]; then
  curl -sSL "https://s3-us-west-1.amazonaws.com/build.nearprotocol.com/nearcore/Linux-x86_64/$SANDBOX_VERSION/near-sandbox.tar.gz" \
    | tar xz -C "$TOOLS" --strip-components=1
fi

# 3. Open-file shim. The sandbox demands a 65535 open-file limit and exits if it
#    cannot set it. Some containers cap the limit lower and forbid raising it.
if [ "$(ulimit -Hn)" != "unlimited" ] && [ "$(ulimit -Hn)" -lt 65535 ] && [ ! -f "$TOOLS/nofile.so" ]; then
  cat > "$TOOLS/nofile.c" <<'C'
#define _GNU_SOURCE
#include <sys/resource.h>
#include <sys/types.h>
#include <dlfcn.h>
/* Test-only: report success when a process asks for a higher open-file limit. */
int setrlimit(__rlimit_resource_t r, const struct rlimit *l) {
    if (r == RLIMIT_NOFILE) return 0;
    int (*real)(__rlimit_resource_t, const struct rlimit *) = dlsym(RTLD_NEXT, "setrlimit");
    return real(r, l);
}
int setrlimit64(__rlimit_resource_t r, const struct rlimit64 *l) {
    if (r == RLIMIT_NOFILE) return 0;
    int (*real)(__rlimit_resource_t, const struct rlimit64 *) = dlsym(RTLD_NEXT, "setrlimit64");
    return real(r, l);
}
int prlimit64(pid_t p, enum __rlimit_resource r, const struct rlimit64 *n, struct rlimit64 *o) {
    int (*real)(pid_t, enum __rlimit_resource, const struct rlimit64 *, struct rlimit64 *) = dlsym(RTLD_NEXT, "prlimit64");
    if (r == RLIMIT_NOFILE && n) return real(p, r, 0, o);
    return real(p, r, n, o);
}
int prlimit(pid_t p, enum __rlimit_resource r, const struct rlimit *n, struct rlimit *o) {
    int (*real)(pid_t, enum __rlimit_resource, const struct rlimit *, struct rlimit *) = dlsym(RTLD_NEXT, "prlimit");
    if (r == RLIMIT_NOFILE && n) return real(p, r, 0, o);
    return real(p, r, n, o);
}
C
  gcc -shared -fPIC -o "$TOOLS/nofile.so" "$TOOLS/nofile.c" -ldl
fi

# 4. Contract code from mainnet, for sandbox tests of the Intents payout.
for acct in intents.near wrap.near; do
  if [ ! -s "$TOOLS/wasm/$acct.wasm" ]; then
    curl -sS https://free.rpc.fastnear.com -H 'content-type: application/json' \
      -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"query\",\"params\":{\"request_type\":\"view_code\",\"finality\":\"final\",\"account_id\":\"$acct\"}}" \
      | python3 -c "import json,sys,base64; r=json.load(sys.stdin)['result']; open('$TOOLS/wasm/$acct.wasm','wb').write(base64.b64decode(r['code_base64']))"
  fi
done

# 5. Docker daemon, for the local Zcash node (Zebra regtest).
if command -v docker >/dev/null && ! docker info >/dev/null 2>&1; then
  if command -v dockerd >/dev/null; then
    (nohup dockerd > "$TOOLS/dockerd.log" 2>&1 &)
    for _ in $(seq 1 20); do docker info >/dev/null 2>&1 && break; sleep 1; done
  fi
fi
docker info >/dev/null 2>&1 && docker pull -q zfnd/zebra:7.0.0-rc.0 >/dev/null || echo "warning: Docker is not available; local Zcash tests will not run"

# 6. Environment for tests. Source this file: `source .tools/env.sh`.
{
  echo "export NEAR_SANDBOX_BIN_PATH=\"$TOOLS/near-sandbox\""
  echo "export ZECRET_WASM_DIR=\"$TOOLS/wasm\""
  [ -f "$TOOLS/nofile.so" ] && echo "export ZECRET_SANDBOX_PRELOAD=\"$TOOLS/nofile.so\""
} > "$TOOLS/env.sh"
echo "ready. Run: source .tools/env.sh"
echo "sandbox tests: LD_PRELOAD=\$ZECRET_SANDBOX_PRELOAD cargo test ..."
