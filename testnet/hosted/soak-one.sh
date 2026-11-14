#!/usr/bin/env bash
# One real swap through the hosted site, recorded like a soak slot.
#   testnet/hosted/soak-one.sh <label> [amount]
set -u
cd "$(dirname "$0")/../.."
n=$1; AMOUNT=${2:-0.01}; OUT=testnet/hosted/soak; LOG=$OUT/results.jsonl
source .tools/env.sh 2>/dev/null || true
export PLAYWRIGHT_BROWSERS_PATH=/opt/pw-browsers ZECRET_BIN=${ZECRET_BIN:-$PWD/.tools/soak/zecret}
s=$(date -u +%s)
timeout 1200 node testnet/hosted/run.mjs "${SOAK_SITE:-https://zecret.chrismg.dev}" .tools/testnet/taker zecret-3f109eea1e.testnet "$AMOUNT" > "$OUT/$n.log" 2>&1
code=$?
cp testnet/hosted/result.json "$OUT/$n.json" 2>/dev/null
python3 - "$n" "$s" "$code" "$OUT/$n.json" "$OUT/$n.log" <<'P' | tee -a "$LOG"
import json,sys,time
n,s,code,rj,lg=sys.argv[1:]
f={}
try:
    r=json.load(open(rj)); f=r.get('facts',{})
    import calendar
    if abs(calendar.timegm(time.strptime(r['startedAt'][:19],'%Y-%m-%dT%H:%M:%S'))-int(s))>120: f={}
except Exception: pass
stage=f.get('finalStage')
try: tail=open(lg,errors='replace').read()[-400:]
except Exception: tail=''
print(json.dumps({'slot':n,'at':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime(int(s))),'secs':int(time.time())-int(s),'exit':int(code),
  'result':'done' if stage=='done' and int(code)==0 else 'FAILED','stage':stage,'title':f.get('doneTitle'),'tail':'' if stage=='done' else tail[-300:]}))
P
