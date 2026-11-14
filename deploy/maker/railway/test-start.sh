#!/bin/sh
# Tests of start.sh (the hosted maker's start script) with a stand-in `zecret` binary.
# Needs root (start.sh prepares the volume as root, then drops to uid 10001), jq and
# setpriv. Touches only a temporary folder. Run from anywhere:
#
#   sh deploy/maker/railway/test-start.sh
#
# Prints one PASS or FAIL line per case and exits 1 if one failed.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
START="$HERE/start.sh"
T="$(mktemp -d)"
chmod 0755 "$T"
trap 'rm -rf "$T"' EXIT
fails=0

# The stand-in binary: prints public-looking values and records how the maker was started.
cat > "$T/zecret" <<'EOF'
#!/bin/sh
case "$1" in
  init) echo "u1standinaddress" ;;
  near-key) echo "ed25519:StandInPublicKey" ;;
  directory) echo '{"ua":"u1standinaddress"}' ;;
  maker) shift; echo "$*" > "$HOME/maker-args" ;;
esac
EOF
chmod 0755 "$T/zecret"

SECRET_KEY="ed25519:SECRETsecretSECRETsecretSECRETsecretSECRETsecretSECRETsecretSECRETsecret"
fill() {
  # The mainnet template with the owner's inputs (the runbook's step) and optional jq edits.
  jq --arg o zecret.near "$1" "$ROOT/crates/zecret-node/config/mainnet-maker.json" \
    | sed 's/OWNER\.near/zecret.near/g'
}
FILLED='.chains.birthday = 3570000 | .price_per_zec = "900000000"'

# run NAME EXPECT(ok|fail) PATTERN [VAR=VALUE ...]: a fresh volume, start.sh with the
# variables; PATTERN must be in the output (ok) or the error (fail).
run() {
  name="$1"; expect="$2"; pattern="$3"; shift 3
  d="$T/$name"; mkdir -p "$d"
  [ -n "${PRESET:-}" ] && eval "$PRESET"
  out="$(env -i PATH="$PATH" ZECRET_DATA="$d" ZECRET_BIN="$T/zecret" ZECRET_IMAGE_CONFIG="$T/image.json" \
    ZECRET_NEW_WALLET=1 ZECRET_NEAR_KEY="$SECRET_KEY" "$@" sh "$START" 2>&1)"
  rc=$?
  ok=1
  if [ "$expect" = ok ] && [ $rc -ne 0 ]; then ok=0; fi
  if [ "$expect" = fail ] && [ $rc -eq 0 ]; then ok=0; fi
  printf '%s\n%s' "$out" "$(cat "$d/maker-args" 2>/dev/null)" | grep -q -- "$pattern" || ok=0
  if printf '%s' "$out" | grep -q "SECRETsecret"; then ok=0; echo "  the secret key was printed"; fi
  if [ $ok = 1 ]; then echo "PASS $name"; else echo "FAIL $name (exit $rc)"; printf '%s\n' "$out" | tail -5 | sed 's/^/  /'; fails=$((fails + 1)); fi
}

# The image's testnet settings, as the Dockerfile writes them.
jq '.chains.birthday = 4614922' "$ROOT/crates/zecret-node/config/testnet-maker.json" > "$T/image.json"

run testnet-default ok "--relay 0.0.0.0:8080"
run testnet-refuses-a-mainnet-image fail "runs with ZECRET_NETWORK=test" ZECRET_IMAGE_CONFIG="$ROOT/crates/zecret-node/config/mainnet-maker.json"
run main-needs-the-second-switch fail "needs ZECRET_ALLOW_MAINNET=1" ZECRET_NETWORK=main
run main-unknown-network fail "must be test or main" ZECRET_NETWORK=mainnet ZECRET_ALLOW_MAINNET=1
run main-needs-a-config fail "ZECRET_MAKER_CONFIG is not set" ZECRET_NETWORK=main ZECRET_ALLOW_MAINNET=1
run main-refuses-the-template fail "placeholder" ZECRET_NETWORK=main ZECRET_ALLOW_MAINNET=1 \
  ZECRET_MAKER_CONFIG="$(base64 -w0 < "$ROOT/crates/zecret-node/config/mainnet-maker.json")"
run main-refuses-price-zero fail "price_per_zec is 0" ZECRET_NETWORK=main ZECRET_ALLOW_MAINNET=1 \
  ZECRET_MAKER_CONFIG="$(fill '.chains.birthday = 3570000' | base64 -w0)"
run main-refuses-no-premium-check fail "premium_check is required" ZECRET_NETWORK=main ZECRET_ALLOW_MAINNET=1 \
  ZECRET_MAKER_CONFIG="$(fill "$FILLED | del(.premium_check)" | base64 -w0)"
run main-refuses-a-low-stipend fail "stipend below 0.06 NEAR" ZECRET_NETWORK=main ZECRET_ALLOW_MAINNET=1 \
  ZECRET_MAKER_CONFIG="$(fill "$FILLED | .stipend = \"30000000000000000000000\"" | base64 -w0)"
run main-refuses-no-birthday fail "chains.birthday" ZECRET_NETWORK=main ZECRET_ALLOW_MAINNET=1 \
  ZECRET_MAKER_CONFIG="$(fill '.price_per_zec = "900000000"' | base64 -w0)"
run main-refuses-http fail "https" ZECRET_NETWORK=main ZECRET_ALLOW_MAINNET=1 \
  ZECRET_MAKER_CONFIG="$(fill "$FILLED | .chains.near_rpc = \"http://rpc.example\"" | base64 -w0)"
run main-refuses-a-testnet-config fail "chains.network is 'test'" ZECRET_NETWORK=main ZECRET_ALLOW_MAINNET=1 \
  ZECRET_MAKER_CONFIG="$(base64 -w0 < "$T/image.json")"
run main-refuses-bad-base64 fail "not JSON\|not base64" ZECRET_NETWORK=main ZECRET_ALLOW_MAINNET=1 ZECRET_MAKER_CONFIG="!!!"
run main-starts-without-relay ok "^--dir " ZECRET_NETWORK=main ZECRET_ALLOW_MAINNET=1 \
  ZECRET_MAKER_CONFIG="$(fill "$FILLED" | base64 -w0)"
# The maker's arguments on mainnet are exactly --dir D (no relay).
if [ "$(cat "$T/main-starts-without-relay/maker-args")" = "--dir $T/main-starts-without-relay" ]; then echo "PASS main-args-have-no-relay"; else echo "FAIL main-args-have-no-relay"; fails=$((fails + 1)); fi
# The NEAR key on the volume is the secret from the variable, owner-only, owned by uid 10001.
k="$T/main-starts-without-relay/near_key.json"
if [ "$(cat "$k")" = "$SECRET_KEY" ] && [ "$(stat -c '%a %u' "$k")" = "600 10001" ]; then echo "PASS main-key-on-volume-0600"; else echo "FAIL main-key-on-volume-0600"; fails=$((fails + 1)); fi

# A later start with other settings while a swap is open keeps the volume's settings.
PRESET='if [ "$name" = main-keeps-settings-with-open-swap ]; then mkdir -p "$d/swap"; fill "$FILLED" > "$d/config.json"; echo "{\"stage\":\"Locked\"}" > "$d/swap/ab.json"; fi'
run main-keeps-settings-with-open-swap ok "1 swap(s) are open" ZECRET_NETWORK=main ZECRET_ALLOW_MAINNET=1 \
  ZECRET_MAKER_CONFIG="$(fill "$FILLED | .max_zats = 1000000" | base64 -w0)"
# With no open swap the new settings replace the old ones, and the old file is kept.
PRESET='if [ "$name" = main-updates-settings-with-no-open-swap ]; then fill "$FILLED" > "$d/config.json"; fi'
run main-updates-settings-with-no-open-swap ok "config.json updated" ZECRET_NETWORK=main ZECRET_ALLOW_MAINNET=1 \
  ZECRET_MAKER_CONFIG="$(fill "$FILLED | .max_zats = 1000000" | base64 -w0)"
# The volume's config.json alone is enough on mainnet.
PRESET='if [ "$name" = main-uses-the-volume-config ]; then fill "$FILLED" > "$d/config.json"; fi'
run main-uses-the-volume-config ok "config.json from the volume" ZECRET_NETWORK=main ZECRET_ALLOW_MAINNET=1
PRESET=

[ $fails = 0 ] && echo "ALL START SCRIPT TESTS PASSED" || echo "$fails START SCRIPT TEST(S) FAILED"
[ $fails = 0 ]
