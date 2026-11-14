#!/usr/bin/env bash
# Soak test: one real swap through the hosted site every 15 minutes (testnet).
#   testnet/hosted/soak.sh <first slot, UTC, e.g. 2026-11-14T03:00:00Z> <slots> [amount]
# Each slot runs testnet/hosted/run.mjs once and appends one JSON line to
# testnet/hosted/soak/results.jsonl. A slot that is already in the file is skipped, so
# the script can be started again after a stop. A slot whose time has fully passed is
# recorded as "missed".
set -u
cd "$(dirname "$0")/../.."
START=$(date -u -d "$1" +%s); SLOTS=$2; AMOUNT=${3:-0.01}
SITE=${SOAK_SITE:-https://zecret.chrismg.dev}
OUT=testnet/hosted/soak; LOG=$OUT/results.jsonl; mkdir -p "$OUT"; touch "$LOG"
source .tools/env.sh 2>/dev/null || true
export PLAYWRIGHT_BROWSERS_PATH=/opt/pw-browsers ZECRET_BIN=${ZECRET_BIN:-$PWD/.tools/soak/zecret}
for i in $(seq 0 $((SLOTS - 1))); do
  n=$(printf '%02d' $((i + 1))); t=$((START + i * 900))
  grep -Eq "\"slot\": ?\"$n\"" "$LOG" && continue
  now=$(date -u +%s)
  if [ "$now" -ge $((t + 900)) ]; then echo "{\"slot\":\"$n\",\"at\":\"$(date -u -d @$t +%FT%TZ)\",\"result\":\"missed\"}" >> "$LOG"; continue; fi
  [ "$now" -lt "$t" ] && sleep $((t - now))
  s=$(date -u +%s)
  timeout 1500 node testnet/hosted/run.mjs "$SITE" .tools/testnet/taker zecret-3f109eea1e.testnet "$AMOUNT" > "$OUT/$n.log" 2>&1
  code=$?
  cp testnet/hosted/result.json "$OUT/$n.json" 2>/dev/null
  python3 - "$n" "$s" "$code" "$OUT/$n.json" "$OUT/$n.log" >> "$LOG" <<'P'
import json,sys,time
n,s,code,rj,lg=sys.argv[1:]
f={}
try:
    r=json.load(open(rj)); f=r.get('facts',{})
    if abs(time.mktime(time.strptime(r['startedAt'][:19],'%Y-%m-%dT%H:%M:%S'))-time.timezone-int(s))>120: f={}
except Exception: pass
stage=f.get('finalStage'); err=''
try:
    tail=open(lg,errors='replace').read()[-600:]
except Exception: tail=''
print(json.dumps({'slot':n,'at':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime(int(s))),'secs':int(time.time())-int(s),'exit':int(code),
  'result':'done' if stage=='done' and int(code)==0 else 'FAILED','stage':stage,'title':f.get('doneTitle'),'tail':'' if stage=='done' else tail[-300:]}))
P
done
echo "{\"slot\":\"end\",\"at\":\"$(date -u +%FT%TZ)\"}" >> "$LOG"
