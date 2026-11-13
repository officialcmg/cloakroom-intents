"""Summarises a quote drift log written by quote_drift.py.

Usage: python3 -I analyse.py quote-drift-2026-11-13.jsonl
"""
import datetime
import json
import statistics
import sys

rows = [json.loads(line) for line in open(sys.argv[1])]
for case in sorted({r["case"] for r in rows}):
    ok = [r for r in rows if r["case"] == case and r.get("amountOut")]
    errors = [r for r in rows if r["case"] == case and not r.get("amountOut")]
    out = [int(r["amountOut"]) for r in ok]
    mins = [int(r["minAmountOut"]) for r in ok]
    ts = [datetime.datetime.fromisoformat(r["t"]) for r in ok]
    print(f"{case}: {len(ok)} quotes, {len(errors)} errors, {ok[0]['t'][11:19]} to {ok[-1]['t'][11:19]} UTC")
    print(f"  amountOut range {(max(out) - min(out)) / statistics.mean(out) * 100:.3f} % of the mean")
    for window in (60, 120, 300, 600):
        worst = max(
            ((out[i] - out[j]) / out[i] * 100
             for i in range(len(ok)) for j in range(i + 1, len(ok))
             if (ts[j] - ts[i]).total_seconds() <= window),
            default=0.0,
        )
        print(f"  worst fall of amountOut within {window} s: {worst:.3f} %")
    below = sum(1 for i in range(len(ok)) for j in range(i + 1, len(ok)) if out[j] < mins[i])
    print(f"  later amountOut below an earlier minAmountOut: {below}")
    print(f"  minAmountOut / amountOut: {sorted({round(m / a, 4) for m, a in zip(mins, out)})}")
    print(f"  latency median {statistics.median(r['ms'] for r in ok)} ms, max {max(r['ms'] for r in ok)} ms")
    for e in errors:
        print(f"  error at {e['t'][11:19]}: {e['err']}")
