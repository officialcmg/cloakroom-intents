#!/bin/sh
# Start script of the hosted maker (deploy/maker/railway/Dockerfile, docs/HOSTING.md).
# It runs as root, prepares the maker folder on the volume, then starts `zecret maker` as
# the unprivileged user zecret (uid 10001).
#
# Two modes. The default is testnet: the settings come from the image (the repository's
# testnet template) and the maker runs with its relay. Mainnet is a separate, deliberate
# mode (docs/MAINNET.md): it needs both ZECRET_NETWORK=main and
# ZECRET_ALLOW_MAINNET=1. On mainnet:
#   - the settings come from ZECRET_MAKER_CONFIG (base64 of config.json; it holds no
#     secret) or from the volume, never from the image;
#   - the settings must say chains.network "main", hold no template placeholder, set
#     price_per_zec, premium_check and chains.birthday, a stipend of at least 0.06 NEAR,
#     and use https endpoints;
#   - the maker runs without the relay (the exit runs through 1Click);
#   - key material comes only from the service's variables or the volume, as on testnet.
#
# The volume is the source of truth. This script never overwrites the wallet, the RFQ
# key, the swap records or the relay counters. It writes:
#   - wallet.json, rfq_key.json, seen.json, swap/ and relay/ from ZECRET_MAKER_STATE
#     (base64 of a tar.gz of a maker folder), only when the volume has no wallet.json,
#     and only the files that are absent;
#   - near_key.json from ZECRET_NEAR_KEY: when absent, or when the variable changed
#     (key rotation; the old file is kept as near_key.json.prev-<time>). Without
#     ZECRET_NEAR_KEY and with ZECRET_NEW_NEAR_KEY=1, a new key is created on the volume
#     and only its public key is printed;
#   - a new wallet with ZECRET_NEW_WALLET=1, when the volume has none and
#     ZECRET_MAKER_STATE is not set;
#   - config.json from the image (testnet) or from ZECRET_MAKER_CONFIG (mainnet): when
#     absent, or when the wanted settings differ and no swap is open. With an open swap it
#     keeps the volume's settings: the maker uses them for its deadlines.
# Secret values are never printed.
set -eu

D="${ZECRET_DATA:-/data}"
IMAGE_CONFIG="${ZECRET_IMAGE_CONFIG:-/etc/zecret/config.testnet.json}"
NETWORK="${ZECRET_NETWORK:-test}"
PORT="${PORT:-8080}"
ZECRET_BIN="${ZECRET_BIN:-/usr/local/bin/zecret}"
UID_ZECRET=10001
log() { echo "[start $(date -u +%Y-%m-%dT%H:%M:%SZ)] $*" >&2; }
die() { log "error: $*"; exit 1; }
stamp() { date -u +%Y%m%dT%H%M%SZ; }

[ "$(id -u)" = 0 ] || die "must start as root to prepare $D (Railway mounts volumes as root)"
case "$NETWORK" in
  test) ;;
  main)
    [ "${ZECRET_ALLOW_MAINNET:-}" = 1 ] || die "ZECRET_NETWORK=main needs ZECRET_ALLOW_MAINNET=1: this service would move real funds"
    log "MAINNET mode: real funds"
    ;;
  *) die "ZECRET_NETWORK must be test or main, not '$NETWORK'" ;;
esac
mkdir -p "$D"
chmod 0700 "$D"
umask 077

# 1. Wallet and the rest of an existing maker folder.
import_state() {
  tmp="$(mktemp -d)"
  if ! printf '%s' "$ZECRET_MAKER_STATE" | base64 -d 2>/dev/null | tar -xz -C "$tmp" --no-same-owner 2>/dev/null; then
    rm -rf "$tmp"
    die "ZECRET_MAKER_STATE is not a base64 tar.gz"
  fi
  # Only the files of a maker folder; no config, no NEAR key, no other paths.
  bad="$(cd "$tmp" && find . -type f | grep -Ev '^\./(wallet\.json|rfq_key\.json|seen\.json|swap/[0-9a-f]+\.json|relay/[0-9a-f]+\.json)$' || true)"
  if [ -n "$bad" ]; then
    rm -rf "$tmp"
    die "ZECRET_MAKER_STATE holds unexpected files: $(echo "$bad" | tr '\n' ' ')"
  fi
  [ -s "$tmp/wallet.json" ] || { rm -rf "$tmp"; die "ZECRET_MAKER_STATE has no wallet.json"; }
  echo "$tmp"
}

if [ ! -e "$D/wallet.json" ]; then
  if [ -n "${ZECRET_MAKER_STATE:-}" ]; then
    tmp="$(import_state)"
    (cd "$tmp" && find . -type f) | while read -r f; do
      f="${f#./}"
      if [ -e "$D/$f" ]; then
        log "kept $f (already on the volume)"
      else
        mkdir -p "$D/$(dirname "$f")"
        cp "$tmp/$f" "$D/$f"
        log "imported $f"
      fi
    done
    rm -rf "$tmp"
  elif [ "${ZECRET_NEW_WALLET:-}" = 1 ]; then
    log "no wallet on the volume; a new one is created below (ZECRET_NEW_WALLET=1)"
  else
    die "no wallet.json in $D and ZECRET_MAKER_STATE is not set (set ZECRET_NEW_WALLET=1 for a new wallet)"
  fi
elif [ -n "${ZECRET_MAKER_STATE:-}" ]; then
  tmp="$(import_state)"
  if cmp -s "$tmp/wallet.json" "$D/wallet.json"; then
    log "wallet on the volume matches ZECRET_MAKER_STATE"
  else
    log "WARNING: the wallet on the volume differs from ZECRET_MAKER_STATE; the volume's wallet is used"
  fi
  rm -rf "$tmp"
fi

# 2. The maker account's NEAR key (one line, ed25519:...).
new_near_key=0
if [ -n "${ZECRET_NEAR_KEY:-}" ]; then
  if [ ! -e "$D/near_key.json" ]; then
    printf '%s' "$ZECRET_NEAR_KEY" > "$D/near_key.json"
    log "wrote near_key.json from ZECRET_NEAR_KEY"
  elif [ "$(cat "$D/near_key.json")" != "$ZECRET_NEAR_KEY" ]; then
    mv "$D/near_key.json" "$D/near_key.json.prev-$(stamp)"
    printf '%s' "$ZECRET_NEAR_KEY" > "$D/near_key.json"
    log "ZECRET_NEAR_KEY changed: wrote near_key.json, kept the old file as near_key.json.prev-*"
  fi
elif [ ! -e "$D/near_key.json" ]; then
  if [ "${ZECRET_NEW_NEAR_KEY:-}" = 1 ]; then
    new_near_key=1
    log "no near_key.json on the volume; a new key is created below (ZECRET_NEW_NEAR_KEY=1)"
  else
    die "no near_key.json in $D and ZECRET_NEAR_KEY is not set (set ZECRET_NEW_NEAR_KEY=1 for a new key)"
  fi
fi

# 3. Settings.
open_swaps=0
if ls "$D"/swap/*.json >/dev/null 2>&1; then
  open_swaps="$(jq -s '[.[] | select((.stage | type) == "string" and .stage != "Swept" and .stage != "Refunded")] | length' "$D"/swap/*.json)"
fi
# The settings this start wants: the image's testnet template, or on mainnet the
# service's ZECRET_MAKER_CONFIG (or nothing: then the volume's config.json is used).
want=""
if [ "$NETWORK" = main ]; then
  if [ -n "${ZECRET_MAKER_CONFIG:-}" ]; then
    want="$(mktemp)"
    printf '%s' "$ZECRET_MAKER_CONFIG" | base64 -d > "$want" 2>/dev/null || die "ZECRET_MAKER_CONFIG is not base64"
    jq -e . "$want" >/dev/null 2>&1 || die "ZECRET_MAKER_CONFIG is not JSON"
  elif [ ! -e "$D/config.json" ]; then
    die "mainnet: no config.json on the volume and ZECRET_MAKER_CONFIG is not set"
  fi
else
  want="$IMAGE_CONFIG"
fi
if [ -z "$want" ]; then
  log "config.json from the volume"
elif [ ! -e "$D/config.json" ]; then
  cp "$want" "$D/config.json"
  log "wrote config.json from $( [ "$NETWORK" = main ] && echo ZECRET_MAKER_CONFIG || echo the image )"
elif jq -e --slurpfile a "$D/config.json" --slurpfile b "$want" -n '$a[0] == $b[0]' >/dev/null; then
  log "config.json matches the wanted settings"
elif [ "$open_swaps" = 0 ]; then
  old="config.json.prev-$(stamp)"
  cp "$D/config.json" "$D/$old"
  cp "$want" "$D/config.json"
  log "config.json updated (no open swap); the old one is $old"
else
  log "WARNING: the wanted settings differ from config.json, but $open_swaps swap(s) are open: kept config.json; it is updated at the first start with no open swap"
fi
[ "$NETWORK" = main ] && [ -n "$want" ] && rm -f "$want"
network="$(jq -r '.chains.network' "$D/config.json")"
[ "$network" = "$NETWORK" ] || die "chains.network is '$network', but this service runs with ZECRET_NETWORK=$NETWORK"
if [ "$NETWORK" = main ]; then
  # Refuse what would lose money or cannot work. Maker::open refuses a missing
  # premium_check too; this stops earlier and says why.
  if grep -Eq 'OWNER|REPLACE' "$D/config.json"; then die "mainnet: config.json still holds a template placeholder (OWNER or REPLACE)"; fi
  jq -e '(.price_per_zec | tostring | tonumber) > 0' "$D/config.json" >/dev/null || die "mainnet: price_per_zec is 0 or missing: set the maker's price (docs/MAINNET.md, the price rule)"
  jq -e '.premium_check.yocto_near_per_zec != null' "$D/config.json" >/dev/null || die "mainnet: premium_check is required"
  # The taker's key sends commit_claim, claim and withdraw(id) from the stipend; below
  # 0.06 NEAR the margin over NEAR's balance check is less than 2
  # (docs/experiments/light-payout.md).
  jq -e '(.stipend | tostring | tonumber) >= 60000000000000000000000' "$D/config.json" >/dev/null || die "mainnet: stipend below 0.06 NEAR: the taker's key could not pay for its claim and payout with a margin of 2"
  jq -e '(.chains.birthday // 0) > 0' "$D/config.json" >/dev/null || die "mainnet: set chains.birthday to a Zcash height before the maker's wallet was made"
  jq -e '(.misbehave // null) == null' "$D/config.json" >/dev/null || die "mainnet: misbehave is a test setting"
  jq -e '(.chains.near_rpc | startswith("https://")) and (.chains.lightwalletd | startswith("https://"))' "$D/config.json" >/dev/null || die "mainnet: near_rpc and lightwalletd must be https URLs"
fi
log "settings: $(jq -c '{network: .chains.network, escrow: .chains.escrow, near_account, birthday: .chains.birthday, min_zats, max_zats, premium_zats, price_per_zec, timing, windows}' "$D/config.json")"
log "open swaps: $open_swaps"

# 4. Ownership, then drop privileges.
chown -R "$UID_ZECRET:$UID_ZECRET" "$D"
find "$D" -type f -exec chmod 0600 {} +
find "$D" -type d -exec chmod 0700 {} +
unset ZECRET_MAKER_STATE ZECRET_NEAR_KEY ZECRET_MAKER_CONFIG

run() { setpriv --reuid="$UID_ZECRET" --regid="$UID_ZECRET" --clear-groups -- env HOME="$D" "$@"; }
# Public values only: the wallet's address, the NEAR public key, the directory entry.
# `init` creates the wallet when it is absent and prints its unified address.
log "wallet address: $(run "$ZECRET_BIN" init --dir "$D")"
if [ "$new_near_key" = 1 ]; then
  log "created near_key.json; add this public key to $(jq -r .near_account "$D/config.json") as a full-access key:"
fi
log "NEAR public key: $(run "$ZECRET_BIN" near-key --dir "$D")"
if entry="$(run "$ZECRET_BIN" directory --dir "$D" 2>&1)"; then
  log "directory entry: $entry"
else
  log "WARNING: zecret directory failed: $entry"
fi
if [ "$NETWORK" = main ]; then
  # No relay on mainnet: the exit runs through 1Click (zecret maker refuses --relay there).
  log "starting: zecret maker --dir $D (mainnet, no relay)"
  exec setpriv --reuid="$UID_ZECRET" --regid="$UID_ZECRET" --clear-groups -- \
    env HOME="$D" "$ZECRET_BIN" maker --dir "$D"
fi
log "starting: zecret maker --dir $D --relay 0.0.0.0:$PORT"
exec setpriv --reuid="$UID_ZECRET" --regid="$UID_ZECRET" --clear-groups -- \
  env HOME="$D" "$ZECRET_BIN" maker --dir "$D" --relay "0.0.0.0:$PORT"
