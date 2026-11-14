#!/usr/bin/env python3
"""Writes testnet/hosted/demo/launch-facts.json: the recorded swap, the edit and every claim
on screen with its source.

    python3 video/make_facts.py <raw dir>
"""
import json
import sys
from pathlib import Path

RAW = Path(sys.argv[1])
HERE = Path(__file__).resolve().parent
OUT = HERE.parent / 'testnet/hosted/demo/launch-facts.json'
facts = json.load(open(RAW / 'facts.json'))
cut = json.load(open(RAW / 'cut.json'))
marks = json.load(open(RAW / 'marks.json'))
pay = next(m['t'] for m in marks if m['name'] == 'pay_sent')
S4 = 36.8 + 0.9  # film time where the footage starts (scene 4 start in src/scenes.js + lead)

claims = [
    ('Shielded ZEC → 30+ chains', '0–5 s', 'README.md (top): "swapping shielded ZEC for tokens on 30+ chains"; web/src/lib/chainNames.ts lists 35 destination chains'),
    ('Swapping ZEC today means unshielding it first. NEAR Intents can already swap ZEC. But that ZEC is a bridged token: it left the shielded pool for a transparent address first.', '8–21 s',
     'README.md "The problem": inside NEAR Intents ZEC is the NEP-141 token nep141:zec.omft.near (1Click token list, 2026-11-13), not shielded ZEC; Cloakroom paper quote on routes through a transparent address'),
    ('The ZEC stays shielded the whole way.', '20–37 s', 'README.md "The problem": "Zecret keeps the ZEC in the shielded pool for the whole swap"; web Landing.tsx privacy section'),
    ('The ZEC goes to a shielded address. Its key is split in two halves. (Your browser / The maker)', '21–37 s', 'README.md top: spending key split in two halves, the user holds one, the maker the other'),
    ('An escrow on NEAR releases the USDC to whoever reveals the right half. The maker locks USDC first.', '26–37 s', 'README.md top and Terms ("Escrow": claim(a) / refund(b)); Landing.tsx FlowDiagram "Locks USDC first"'),
    ('The USDC lands in NEAR Intents under a key made in your browser (gas from the maker). One signature sends it on.', '31–37 s', 'README.md top; Terms "Stipend" (0.06 NEAR from the maker to the taker\'s new NEAR key)'),
    ('35 chains in the swap form (chain names, each with its logo)', '33–37 s', 'web/src/lib/chainNames.ts (35 entries); logos from web/src/ui/chainIconData.ts, sources and licences in web/src/assets/chains/SOURCES.md'),
    ('Live on Zcash testnet and NEAR testnet · test money', '37–67 s (badge)', 'README.md Status; visible in footage (site banner and network tag)'),
    ('Runs in your browser. No install, no account.', 'scene 4', 'README.md Terms ("Taker ... runs in the user\'s browser"); Landing.tsx; the footage shows no sign-in or install'),
    ('Pay once from any shielded wallet', 'scene 4', 'Landing.tsx step 2; footage: one payment to the shown address (paid here by the command-line testnet wallet, as in testnet/hosted/record-demo.mjs)'),
    ('The maker locks USDC first', 'scene 4', 'footage: the step "The maker locks the USDC" completes before "Your ZEC goes to the shared address"'),
    ('Claimed with a key made here', 'scene 4', 'footage: step "You claim the USDC"; README.md Terms ("Key half", "Stipend")'),
    ('Delivered', 'scene 4', f'footage: "{facts.get("doneTitle", "")}"'),
    (f'Real time: {int(cut["paymentToFinalSecs"] // 60)} min {round(cut["paymentToFinalSecs"] % 60)} s from payment to done', 'scene 4', 'this recording, marks.json (pay_sent to stage done)'),
    ('Real swaps on public test networks. 32 real swaps completed; 35 destination chains in the swap form', '66–79 s', 'README.md Status ("32 completed"); web/src/lib/chainNames.ts (35 chains). Counts as of 2026-11-14, before this recording.'),
    ('6–9 minutes per swap, mostly waiting for Zcash blocks', '66–79 s', 'docs/TESTNET.md soak: 12 back-to-back swaps took 364 to 574 s; README.md hosted runs 314–493 s'),
    ('0 bridges or custodians holding the ZEC', '66–79 s', 'README.md top: the ZEC goes to a shielded address spent only with both key halves'),
    ('Exit to other chains: built. The live mainnet contract accepts its signed message in a read-only simulation. Not yet run with funds.', '70–79 s', 'README.md Status, "Exit through NEAR Intents on mainnet" and the brief\'s fact list; docs/INTENTS.md'),
    ('Shielded ZEC to 30+ chains. No bridge holds your coins.', '78–88 s', 'as above'),
    ('cloakroom.chrismg.dev', '78–88 s', 'the live site, which showed "Cloakroom Intents" at the recording'),
    ('Testnet today · Built on Cloakroom · NEAR HACKATHON 2026', '78–88 s', 'README.md: testnet only, mainnet not deployed; built on Cloakroom (commit 2f8e2f7); NEAR HACKATHON 2026-11-01 to 2026-11-14'),
]

out = {
    'film': {'file': 'cloakroom-intents-launch.mp4', 'seconds': 87.6, 'size': '1920x1080', 'fps': 30, 'audio': 'silent AAC'},
    'swap': {k: facts.get(k) for k in ('site', 'amountZec', 'nearAccount', 'banner', 'preview', 'payTx', 'finalStage', 'doneTitle', 'txs', 'escrow', 'payout', 'exit', 'lastError', 'history', 'startedAt', 'finishedAt')},
    'realDuration': {'paymentToDoneSecs': cut['paymentToFinalSecs']},
    'edit': {
        'footageStartsAtFilmSecs': S4,
        'segments': [dict(s, filmFromSecs=round(S4 + o, 2)) for s, o in zip(cut['segments'], [cut['filmEvents'][s['kind']] for s in cut['segments']])],
        'note': 'speed is source seconds per film second. The clock in the film shows the true time since the payment on every frame after it.',
    },
    'claims': [{'text': t, 'when': w, 'source': s} for t, w, s in claims],
}
json.dump(out, open(OUT, 'w'), indent=2, ensure_ascii=False)
print('wrote', OUT)
