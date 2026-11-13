#!/bin/sh
# Start script of the hosted testnet maker (deploy/maker/railway/Dockerfile,
# docs/HOSTING.md). It runs as root, prepares the maker folder on the volume, then
# starts `zecret maker` with its relay as the unprivileged user zecret (uid 10001).
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
#   - config.json from the image (the repository's testnet template): when absent, or
#     when the image's settings differ and no swap is open. With an open swap it keeps
#     the volume's settings: the maker uses them for its deadlines.
# Secret values are never printed.
set -eu

D="${ZECRET_DATA:-/data}"
IMAGE_CONFIG="${ZECRET_IMAGE_CONFIG:-/etc/zecret/config.testnet.json}"
PORT="${PORT:-8080}"
ZECRET_BIN="${ZECRET_BIN:-/usr/local/bin/zecret}"
UID_ZECRET=10001
log() { echo "[start $(date -u +%Y-%m-%dT%H:%M:%SZ)] $*" >&2; }
die() { log "error: $*"; exit 1; }
stamp() { date -u +%Y%m%dT%H%M%SZ; }

[ "$(id -u)" = 0 ] || die "must start as root to prepare $D (Railway mounts volumes as root)"
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
if [ ! -e "$D/config.json" ]; then
  cp "$IMAGE_CONFIG" "$D/config.json"
  log "wrote config.json from the image"
elif jq -e --slurpfile a "$D/config.json" --slurpfile b "$IMAGE_CONFIG" -n '$a[0] == $b[0]' >/dev/null; then
  log "config.json matches the image"
elif [ "$open_swaps" = 0 ]; then
  old="config.json.prev-$(stamp)"
  cp "$D/config.json" "$D/$old"
  cp "$IMAGE_CONFIG" "$D/config.json"
  log "config.json updated from the image (no open swap); the old one is $old"
else
  log "WARNING: the image's settings differ from config.json, but $open_swaps swap(s) are open: kept config.json; it is updated at the first start with no open swap"
fi
network="$(jq -r '.chains.network' "$D/config.json")"
[ "$network" = test ] || die "chains.network is '$network': this service runs on testnet only"
log "settings: $(jq -c '{escrow: .chains.escrow, near_account, birthday: .chains.birthday, min_zats, max_zats, premium_zats, price_per_zec, timing, windows}' "$D/config.json")"
log "open swaps: $open_swaps"

# 4. Ownership, then drop privileges.
chown -R "$UID_ZECRET:$UID_ZECRET" "$D"
find "$D" -type f -exec chmod 0600 {} +
find "$D" -type d -exec chmod 0700 {} +
unset ZECRET_MAKER_STATE ZECRET_NEAR_KEY

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
log "starting: zecret maker --dir $D --relay 0.0.0.0:$PORT"
exec setpriv --reuid="$UID_ZECRET" --regid="$UID_ZECRET" --clear-groups -- \
  env HOME="$D" "$ZECRET_BIN" maker --dir "$D" --relay "0.0.0.0:$PORT"
