#!/usr/bin/env bash
# The owner's first funded test of the exit leg (NEAR Intents 1Click) on NEAR mainnet,
# and the rescue of a payout with only a recovery file. docs/MAINNET.md, "The exit leg:
# first funded test", says when and how to run it.
#
# Read-only (sends nothing, spends nothing):
#   scripts/mainnet-exit-test.sh plan   --owner-key KEY.json --amount 1000000 \
#       --dest-asset nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near --recipient 0xYOURADDRESS
#   scripts/mainnet-exit-test.sh status --state ./exit-test-state/exit-test-<id>.json
#   scripts/mainnet-exit-test.sh rescue --recovery-file zecret-recovery-xxxx.json --to-near-account you.near
#       (without the flag it only signs and simulates)
#
# Spends real funds (needs --i-understand-this-spends-real-funds):
#   run     deposits --amount USDC from the owner's account into a fresh key's NEAR Intents
#           account and runs the engine's exit code to --recipient. The key's seed is stored in
#           --state-dir (default ./exit-test-state, mode 0600) before anything is sent.
#   refund  moves a test key's USDC back to the owner's NEAR account (the owner pays the gas).
#   rescue  sends the simulated rescue (the swap key's own NEAR account pays the gas, or
#           --gas-payer-key KEY.json does).
#
# KEY.json is a near-cli credentials file: {"account_id": ..., "private_key": "ed25519:..."}.
# --amount is in USDC base units (6 decimals); the cap is 5 USDC unless --max-amount says more.
# Options: --rpc URL (default https://rpc.mainnet.fastnear.com), --min-out N or
# --headroom-bps N (default 50) for the approved minimum, --oneclick URL.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FLAG=--i-understand-this-spends-real-funds
cmd="${1:-}"
case "$cmd" in
  plan | status) ;;
  rescue) ;;
  run | refund)
    spend=0
    for a in "$@"; do [ "$a" = "$FLAG" ] && spend=1; done
    if [ "$spend" != 1 ]; then
      echo "refused: '$cmd' spends real funds on NEAR mainnet. Run 'plan' first, then add $FLAG." >&2
      exit 2
    fi
    ;;
  *)
    sed -n '2,27p' "$0" | sed 's/^# \{0,1\}//'
    exit 2
    ;;
esac
[ -f "$ROOT/.tools/env.sh" ] && . "$ROOT/.tools/env.sh" >/dev/null 2>&1 || true
# Relative paths (--state-dir, key files) stay relative to where the script was started.
exec cargo run -q --manifest-path "$ROOT/crates/Cargo.toml" -p zecret-wasm --example mainnet_exit -- "$@"
