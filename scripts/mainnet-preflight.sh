#!/usr/bin/env bash
# Mainnet preflight: every check that needs no funds, in one run (docs/MAINNET.md).
# It sends no transaction and spends nothing. It prints one PASS or FAIL line per check,
# the log of a failed check under it, and a summary. Exit 1 if a check failed.
#
#   OWNER=zecret.near ESCROW_HASH=<reviewed code hash> MAKER_CONFIG=/path/config.json \
#     scripts/mainnet-preflight.sh [--quick]
#
# Inputs (environment):
#   OWNER         the owner's NEAR mainnet account. Unset: the dry run uses the existing
#                 account "aurora" as a stand-in owner (read-only), and says so.
#   ESCROW_HASH   the code hash of the reviewed escrow build (zecret-deploy hash FILE).
#   ESCROW_WASM   the escrow wasm to deploy. Default: the build in contracts/near-escrow.
#   MAKER_CONFIG  the mainnet maker's config.json (no key in it). Optional.
#   SITE_ORIGIN   the mainnet site's origin, for the CORS check of the light-wallet server.
#                 Default https://zecret.chrismg.dev.
#   DEPLOY_TARGET_DIR, CRATES_TARGET_DIR  cargo target folders for deploy/ and crates/,
#                 to reuse earlier builds and save disk (optional).
#   SKIP_ENGINE_BUILD=1  use web/public/engine as it is (it must be built from this checkout).
# --quick skips the cargo tests, the web build and the Playwright tests.
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
QUICK=0
[ "${1:-}" = --quick ] && QUICK=1
LOG="$(mktemp -d)"
trap 'rm -rf "$LOG"' EXIT
pass=0
fail=0
n=0

check() {
  # check NAME COMMAND...: runs the command in a subshell, PASS on exit 0.
  local name="$1"
  shift
  n=$((n + 1))
  if (cd "$ROOT" && eval "$@") >"$LOG/$n" 2>&1; then
    echo "PASS $name"
    pass=$((pass + 1))
  else
    echo "FAIL $name"
    tail -n 15 "$LOG/$n" | sed 's/^/     | /'
    fail=$((fail + 1))
  fi
}

[ -f "$ROOT/.tools/env.sh" ] && . "$ROOT/.tools/env.sh"
unset CARGO_TARGET_DIR
DT="${DEPLOY_TARGET_DIR:+CARGO_TARGET_DIR=$DEPLOY_TARGET_DIR }"
CT="${CRATES_TARGET_DIR:+CARGO_TARGET_DIR=$CRATES_TARGET_DIR }"
SITE_ORIGIN="${SITE_ORIGIN:-https://zecret.chrismg.dev}"
ESCROW_WASM_GIVEN="${ESCROW_WASM:-}"
ESCROW_WASM="${ESCROW_WASM:-$ROOT/contracts/near-escrow/target/wasm32-unknown-unknown/release/near_escrow.wasm}"
LWD="$(jq -r .zcash.lightwalletd "$ROOT/web/public/config/mainnet.json")"
RPC="$(jq -r .near.rpc "$ROOT/web/public/config/mainnet.json")"
ONECLICK="$(jq -r .exit.oneClickUrl "$ROOT/web/public/config/mainnet.json")"
echo "== Zecret mainnet preflight $(date -u +%Y-%m-%dT%H:%M:%SZ) (nothing is sent)"

# 1. Tools.
check "tools: cargo, node, npm, jq, curl, python3" 'command -v cargo && command -v node && command -v npm && command -v jq && command -v curl && command -v python3'

# 2. Configuration files.
check "configs: mainnet templates parse and name intents.near and Circle's USDC" '
  for f in crates/zecret-node/config/mainnet-maker.json crates/zecret-node/config/mainnet-taker.json web/public/config/mainnet.json; do jq -e . "$f" >/dev/null; done &&
  jq -e ".chains.intents == \"intents.near\" and .chains.token == \"17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1\" and .chains.network == \"main\"" crates/zecret-node/config/mainnet-maker.json &&
  jq -e ".near.verifier == \"intents.near\" and .exit.mode == \"oneclick\" and .near.escrow == \"@build\"" web/public/config/mainnet.json &&
  grep -q "^network = \"mainnet\"" deploy/configs/mainnet.toml'
check "configs: testnet files unchanged in meaning (testnet network, direct exit, relay)" '
  jq -e ".network == \"testnet\" and .exit.mode == \"direct\"" web/public/config/testnet.json &&
  jq -e ".chains.network == \"test\"" crates/zecret-node/config/testnet-maker.json'
if [ -n "${MAKER_CONFIG:-}" ]; then
  check "maker config: no placeholder, price, premium_check, birthday, https, main" "
    ! grep -Eq 'OWNER|REPLACE' '$MAKER_CONFIG' &&
    jq -e '.chains.network == \"main\" and ((.price_per_zec|tostring|tonumber) > 0) and .premium_check.yocto_near_per_zec != null and (.chains.birthday // 0) > 0 and (.chains.near_rpc|startswith(\"https://\")) and (.chains.lightwalletd|startswith(\"https://\"))' '$MAKER_CONFIG'"
fi

# 3. Tests.
if [ $QUICK = 0 ]; then
  check "deploy tool: cargo test (mainnet guard, plan, costs, price rule)" "cd deploy && ${DT}cargo test -p zecret-deploy"
  check "deploy tool: cargo clippy -D warnings" "cd deploy && ${DT}cargo clippy -p zecret-deploy --all-targets -- -D warnings"
  check "zecret-node: mainnet and testnet config tests (paper bounds, premium check)" "cd crates && ${CT}cargo test -p zecret-node --lib config"
  check "zecret-core: mainnet consensus parameters (NU7 at 3,543,405)" "cd crates && ${CT}cargo test -p zecret-core --lib mainnet_parameters"
  check "maker start script: mainnet mode tests" 'sh deploy/maker/railway/test-start.sh'
  # The browser engine of this checkout: an engine built from code without Zcash mainnet
  # parameters fails the browser check below. SKIP_ENGINE_BUILD=1 keeps web/public/engine.
  if [ -z "${SKIP_ENGINE_BUILD:-}" ]; then
    check "web: build the browser engine (crates/zecret-wasm/build.sh)" 'CC_wasm32_unknown_unknown=clang AR_wasm32_unknown_unknown=llvm-ar crates/zecret-wasm/build.sh'
  fi
  check "web: npm ci" 'cd web && npm ci --no-audit --no-fund'
  check "web: typecheck" 'cd web && npm run typecheck'
  check "web: lint" 'cd web && npm run lint'
  check "web: unit tests (CSP from the configs, mainnet switch)" 'cd web && npm test'
  check "web: default build" 'cd web && npm run build'
  check "web e2e: the default build refuses mainnet" 'cd web && npx playwright test --trace off e2e/mainnet-off.spec.ts'
  check "web e2e: the switched build runs mainnet with its notices" 'cd web && npx playwright test -c playwright.mainnet.config.ts'
fi
check "web: CSP connect-src lists the mainnet hosts" "jq -r '.headers[0].headers[0].value' web/vercel.json | grep -q '$LWD' && jq -r '.headers[0].headers[0].value' web/vercel.json | grep -q '$RPC'"

# 4. Endpoints.
check "NEAR RPC $RPC answers with chain_id mainnet" "curl -sS -m 20 '$RPC' -H 'content-type: application/json' -d '{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"status\",\"params\":[]}' | jq -e '.result.chain_id == \"mainnet\"'"
check "Zcash light-wallet $LWD: gRPC-web, chain main, branch NU7" "printf '\\0\\0\\0\\0\\0' | curl -sS -m 20 -X POST '$LWD/cash.z.wallet.sdk.rpc.CompactTxStreamer/GetLightdInfo' -H 'content-type: application/grpc-web+proto' -H 'x-grpc-web: 1' --data-binary @- > \"\$LOG/lwd\" && grep -aq 'grpc-status: *0' \"\$LOG/lwd\" && grep -aq 77190ad9 \"\$LOG/lwd\" && grep -aq main \"\$LOG/lwd\""
check "Zcash light-wallet CORS allows $SITE_ORIGIN" "curl -sS -m 20 -o /dev/null -D - -X OPTIONS '$LWD/cash.z.wallet.sdk.rpc.CompactTxStreamer/GetLightdInfo' -H 'Origin: $SITE_ORIGIN' -H 'Access-Control-Request-Method: POST' -H 'Access-Control-Request-Headers: content-type,x-grpc-web' | grep -iq 'access-control-allow-origin: \\($SITE_ORIGIN\\|\\*\\)'"
check "1Click $ONECLICK/v0/tokens lists USDC on NEAR" "curl -sS -m 30 '$ONECLICK/v0/tokens' | jq -e 'map(select(.assetId == \"nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1\" and .decimals == 6)) | length == 1'"
if [ $QUICK = 0 ]; then
  check "browser: light-wallet, engine tip, NEAR RPC and 1Click from a page" 'cd web && node ../scripts/mainnet-browser-check.mjs'
fi

# 5. The dry run.
OWNER_ARG="${OWNER:-aurora}"
plan="cd deploy && ${DT}cargo run -q -p zecret-deploy -- plan --config configs/mainnet.toml --owner '$OWNER_ARG' --escrow-wasm '$ESCROW_WASM'"
[ -n "${ESCROW_HASH:-}" ] && plan="$plan --expect-escrow-hash '$ESCROW_HASH'"
[ -n "${MAKER_CONFIG:-}" ] && plan="$plan --maker-config '$MAKER_CONFIG'"
if [ $QUICK = 0 ] && [ -z "${ESCROW_WASM_GIVEN:-}" ]; then
  # The escrow's own target folder (its .cargo/config.toml sets the wasm flags).
  check "escrow: build the wasm from this checkout" 'cd contracts/near-escrow && cargo build --target wasm32-unknown-unknown --release'
fi
check "escrow wasm exists ($ESCROW_WASM)" "test -s '$ESCROW_WASM'"
if [ -n "${OWNER:-}" ]; then what="owner $OWNER"; else what="stand-in owner aurora, read-only; set OWNER for the real check"; fi
check "zecret-deploy plan on mainnet ($what)" "$plan"
sed -n '/^== dry run/,$p' "$LOG/$n" | sed 's/^/     /'

echo "== $pass passed, $fail failed"
[ $fail = 0 ] && echo "PREFLIGHT PASSED (no funds used; this does not test anything that needs funds: docs/MAINNET.md, gap list)" || echo "PREFLIGHT FAILED"
[ $fail = 0 ]
