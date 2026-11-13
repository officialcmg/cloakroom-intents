#!/usr/bin/env python3
"""Cuts the demo video from one recording made by testnet/hosted/record-demo.mjs.

    python3 testnet/hosted/demo/edit_demo.py <raw dir> <out dir>

Reads <raw dir>/raw.mkv (frame times are Unix seconds), marks.json and facts.json.
Writes zecret-demo.mp4, zecret-demo-poster.jpg and demo-facts.json to <out dir>.

Everything before the payment plays at real speed, except two waits for the page
(the engine loading and the key generation), which are sped up and labelled. After
the payment, every stage change plays at real speed for about two seconds; the time
between stage changes is sped up by one factor, chosen so the progress part lasts
about PROGRESS_TARGET seconds. A label in the top centre shows the speed and the true
time since the payment whenever the video is not at real speed.
"""
import json
import math
import subprocess
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFont

RAW, OUT = Path(sys.argv[1]), Path(sys.argv[2])
OUT.mkdir(parents=True, exist_ok=True)
W, H, FPS = 1920, 1080, 30
PROGRESS_TARGET = 30.0
GREEN = (10, 58, 47)  # --green-900 in web/src/styles/tokens.css
FONT_DIR = '/usr/share/fonts/opentype/inter/'
F_CAP = ImageFont.truetype(FONT_DIR + 'Inter-SemiBold.otf', 40)
F_TAG = ImageFont.truetype(FONT_DIR + 'Inter-SemiBold.otf', 27)
F_END1 = ImageFont.truetype(FONT_DIR + 'Inter-SemiBold.otf', 58)
F_END2 = ImageFont.truetype(FONT_DIR + 'Inter-Medium.otf', 38)

marks = json.load(open(RAW / 'marks.json'))
facts = json.load(open(RAW / 'facts.json'))


def m(name, default=None):
    for x in marks:
        if x['name'] == name:
            return x['t']
    if default is None:
        raise KeyError(name)
    return default


pts = [float(l.strip().strip(',')) for l in subprocess.check_output(
    ['ffprobe', '-v', 'error', '-select_streams', 'v', '-show_entries', 'frame=pts_time', '-of', 'csv=p=0', str(RAW / 'raw.mkv')],
    text=True).split() if l.strip()]
print(f'raw: {len(pts)} frames, {pts[-1] - pts[0]:.1f} s')

# ---- the timeline: (source start, source end, speed, kind) ------------------------------
segs = []


def seg(a, b, speed=1.0, kind='real'):
    if b - a > 0.02:
        segs.append([a, b, speed, kind])


def squeeze(a, b, out_secs, kind):
    """Plays [a, b] in about out_secs, or at real speed if it is shorter."""
    d = b - a
    seg(a, b, max(1.0, d / out_secs), kind if d > out_secs * 1.05 else 'real')


pay_sent = m('pay_sent')
final = m('final_screen')
end = m('end') if facts.get('finalStage') == 'done' else min(m('end'), m('final_screen') + 7.0)
UI = 1.5  # form filling and clicking between screens, labelled "sped up 1.5x"
t_land = m('scroll_start') - 3.2                                        # the hero for about 3 s
seg(t_land, m('open_swap') + 0.3)
squeeze(m('open_swap') + 0.3, m('form_ready'), 1.2, 'wait')            # the engine loads
seg(m('form_ready'), m('paste_return') + 0.6, UI, 'ui')
squeeze(m('paste_return') + 0.6, m('price_wait_end'), 1.0, 'wait')      # the price arrives
seg(m('price_wait_end'), m('continue') + 0.4)
squeeze(m('continue') + 0.4, m('recovery_shown'), 1.6, 'wait')          # keys are made
seg(m('recovery_shown'), m('pay_screen') - 0.3, UI, 'ui')
seg(m('pay_screen') - 0.3, m('pay_start'))
squeeze(m('pay_start'), pay_sent, 1.5, 'paying')                        # the wallet builds the payment

# Progress: real speed around each stage change, one speed factor between them.
changes = sorted(x['t'] for x in marks if x['name'].startswith('stage:') and pay_sent < x['t'] <= final)
wins = []
for c in changes:
    a, b = max(pay_sent, c - 0.5), min(final, c + 1.6)
    if wins and a <= wins[-1][1] + 0.6:
        wins[-1][1] = max(wins[-1][1], b)
    else:
        wins.append([a, b])
real_total = sum(b - a for a, b in wins)
gap_total = (final - pay_sent) - real_total
factor = max(4.0, gap_total / max(4.0, PROGRESS_TARGET - real_total))
factor = float(round(factor))
cur = pay_sent
for a, b in wins:
    if a > cur:
        seg(cur, a, factor, 'fast')
    seg(max(a, cur), b, 1.0, 'progress-real')
    cur = b
if final > cur:
    seg(cur, final, factor, 'fast')
seg(final, end + 0.3)

# Output time of each source time (for the captions).
out_t, starts = 0.0, []
for s in segs:
    starts.append(out_t)
    out_t += (s[1] - s[0]) / s[2]
END_CARD = 3.0
XFADE = 0.5
total = out_t + END_CARD


def at(src):
    for s, o in zip(segs, starts):
        if s[0] <= src <= s[1]:
            return o + (src - s[0]) / s[2]
    if src < segs[0][0]:
        return 0.0
    return out_t


real_minutes = (m('stage:done', m('stage:' + str(facts.get('finalStage')), final)) - pay_sent) / 60
claim_t = m('stage:claiming', m('stage:committing', final - 20))
captions = [
    (at(t_land), at(t_land) + 5.0, 'Zecret on testnet: shielded ZEC to tokens on other chains, no custodian'),
    (at(t_land) + 5.4, at(m('open_swap')), 'Runs on Zcash testnet and NEAR testnet today (test money)'),
    (at(m('open_swap')) + 0.2, at(m('form_ready')) + 2.6, 'One real swap on the live site, start to finish'),
    (at(m('form_ready')) + 3.0, at(m('price_shown')), 'Enter the amount and where the tokens go'),
    (at(m('price_shown')), at(m('continue')), 'On testnet, test USDC goes to a NEAR testnet account'),
    (at(m('recovery_shown')), at(m('pay_screen')) - 0.2, 'Save the recovery file'),
    (at(m('pay_screen')), at(pay_sent) + 2.0, 'Pay once, from any shielded wallet (here: a testnet wallet)'),
    (at(pay_sent) + 2.4, at(pay_sent) + 11.0, 'The browser runs the swap: encrypted memos, an escrow on NEAR'),
    (at(pay_sent) + 11.4, at(claim_t) - 0.6, f'Real time: about {round(real_minutes)} minutes. Sped up here.'),
    (at(claim_t) - 0.2, at(final) - 0.2, 'The USDC is claimed with a key made in this browser'),
    (at(final), out_t, 'Done: test USDC arrived on NEAR testnet'),
]
if facts.get('finalStage') != 'done':
    # Not a finished swap: say what happened, plainly.
    err = (facts.get('lastError') or {}).get('code', '')
    ending = ('The maker did not answer in time. The ZEC went back to the return address.' if err == 'MAKER_TIMEOUT'
              else f'The swap ended in stage "{facts.get("finalStage")}" ({err or "no error code"})')
    captions = captions[:7] + [
        (at(pay_sent) + 2.4, at(pay_sent) + 11.0, 'The browser sends the maker an encrypted request (a Zcash memo)'),
        (at(pay_sent) + 11.4, at(final) - 0.2, f'Real time: about {round(real_minutes)} minutes. Sped up here.'),
        (at(final), out_t, ending),
    ]


# ---- drawing ------------------------------------------------------------------------------
def pill(text, font, pad_x, pad_y, fill=(12, 20, 17, 190), color=(255, 255, 255, 255)):
    l, t, r, b = font.getbbox(text)
    w, h = r - l + 2 * pad_x, font.size + 2 * pad_y
    im = Image.new('RGBA', (w, h), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    d.rounded_rectangle((0, 0, w - 1, h - 1), radius=h // 2, fill=fill)
    d.text((pad_x - l, pad_y + (font.size - (b + t)) / 2), text, font=font, fill=color)
    return im


cap_imgs = [pill(c[2], F_CAP, 40, 22) for c in captions]
for im, c in zip(cap_imgs, captions):
    assert im.width < W - 120, f'caption too wide: {c[2]}'
tag_cache = {}


def tag(text):
    if text not in tag_cache:
        im = pill(text, F_TAG, 26, 14, fill=(12, 20, 17, 205))
        # a small fast-forward mark before the text
        d = ImageDraw.Draw(im)
        tag_cache[text] = im
    return tag_cache[text]


def end_card():
    im = Image.new('RGB', (W, H), GREEN)
    d = ImageDraw.Draw(im)
    l1 = 'zecret.chrismg.dev  ·  github.com/officialcmg/zecret'
    l2 = 'Runs on Zcash testnet and NEAR testnet (test money)'
    for text, font, y, col in ((l1, F_END1, H / 2 - 50, (255, 255, 255)), (l2, F_END2, H / 2 + 40, (190, 225, 210))):
        w = d.textlength(text, font=font)
        d.text(((W - w) / 2, y), text, font=font, fill=col)
    return im


END_IMG = end_card()


def overlay_text(src, s):
    kind, speed = s[3], s[2]
    if kind == 'real':
        return None
    if kind == 'ui':
        return f'Sped up {speed:.1f}×'
    if kind == 'wait':
        return f'Waiting for the page · sped up {speed:.0f}×'
    if kind == 'paying':
        return f'Wallet building the payment · sped up {speed:.0f}×'
    e = max(0, src - pay_sent)
    clock = f'{int(e // 60):02d}:{int(e % 60):02d} since payment'
    if kind == 'fast':
        return f'{clock} · sped up {speed:.0f}×'
    return f'{clock} · real speed'


def compose(frame, o, src, s):
    im = Image.fromarray(frame).convert('RGBA')
    txt = overlay_text(src, s) if s else None
    if txt:
        t = tag(txt)
        im.alpha_composite(t, ((W - t.width) // 2, 34))
    for (a, b, _), ci in zip(captions, cap_imgs):
        if a <= o < b:
            alpha = min(1.0, (o - a) / 0.25, (b - o) / 0.25)
            c = ci if alpha >= 1 else Image.blend(Image.new('RGBA', ci.size, (0, 0, 0, 0)), ci, alpha)
            im.alpha_composite(c, ((W - c.width) // 2, H - c.height - 56))
    return im.convert('RGB')


# ---- rendering ------------------------------------------------------------------------------
dec = subprocess.Popen(['ffmpeg', '-v', 'fatal', '-i', str(RAW / 'raw.mkv'), '-fps_mode', 'passthrough', '-vf', 'setpts=N/30/TB', '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-'],
                       stdout=subprocess.PIPE, bufsize=W * H * 3 * 4)
enc = subprocess.Popen(['ffmpeg', '-y', '-v', 'error', '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-s', f'{W}x{H}', '-r', str(FPS), '-i', '-',
                        '-f', 'lavfi', '-i', 'anullsrc=channel_layout=stereo:sample_rate=48000',
                        '-map', '0:v', '-map', '1:a', '-shortest',
                        '-c:v', 'libx264', '-preset', 'slow', '-crf', '20', '-pix_fmt', 'yuv420p', '-profile:v', 'high',
                        '-c:a', 'aac', '-b:a', '32k', '-movflags', '+faststart', str(OUT / 'zecret-demo.mp4')], stdin=subprocess.PIPE)
FS = W * H * 3
idx, frame = -1, None


def frame_at(t):
    global idx, frame
    while idx + 1 < len(pts) and pts[idx + 1] <= t + 1e-6:
        buf = dec.stdout.read(FS)
        if len(buf) < FS:
            break
        idx += 1
        frame = np.frombuffer(buf, np.uint8).reshape(H, W, 3)
    if frame is None:
        buf = dec.stdout.read(FS)
        idx = 0
        frame = np.frombuffer(buf, np.uint8).reshape(H, W, 3)
    return frame


n_out = int(round(total * FPS))
poster_t = at(final) + 2.0
last = None
for k in range(n_out):
    o = k / FPS
    if o < out_t:
        i = max(j for j in range(len(segs)) if starts[j] <= o + 1e-9)
        s = segs[i]
        src = s[0] + (o - starts[i]) * s[2]
        img = compose(frame_at(src), o, src, s)
        last = img
        if abs(o - poster_t) < 0.5 / FPS:
            Image.fromarray(np.asarray(frame_at(src))).save(OUT / 'zecret-demo-poster.jpg', quality=92)
    else:
        e = o - out_t
        img = Image.blend(last, END_IMG, min(1.0, e / XFADE))
    enc.stdin.write(img.tobytes())
enc.stdin.close()
enc.wait()
rest = 0
while True:
    b = dec.stdout.read(FS)
    if len(b) < FS:
        break
    rest += 1
dec.wait()
assert idx + 1 + rest == len(pts), f'decoded {idx + 1 + rest} frames, expected {len(pts)}'

out = {
    'swap': {k: facts.get(k) for k in ('site', 'amountZec', 'nearAccount', 'banner', 'preview', 'payTx', 'finalStage', 'doneTitle', 'txs', 'escrow', 'payout', 'exit', 'lastError', 'history', 'engineTimings', 'startedAt', 'finishedAt')},
    'realDuration': {
        'paymentSentToFinalStageSecs': round(real_minutes * 60, 1),
        'paymentSentToFinalStageMinutes': round(real_minutes, 2),
        'paymentSentAt': pay_sent,
        'recordingSecs': round(pts[-1] - pts[0], 1),
    },
    'stageChangesSeenByScript': [{'stage': x['name'][6:], 'secsAfterPayment': round(x['t'] - pay_sent, 1)} for x in marks if x['name'].startswith('stage:')],
    'segments': [{'sourceFromSecs': round(s[0] - pts[0], 2), 'sourceToSecs': round(s[1] - pts[0], 2), 'speed': round(s[2], 2), 'kind': s[3],
                  'outputFromSecs': round(o, 2), 'outputSecs': round((s[1] - s[0]) / s[2], 2)} for s, o in zip(segs, starts)],
    'progressSpeedFactor': factor,
    'captions': [{'fromSecs': round(a, 2), 'toSecs': round(b, 2), 'text': t} for a, b, t in captions],
    'endCardSecs': END_CARD,
    'outputSecs': round(total, 2),
}
json.dump(out, open(OUT / 'demo-facts.json', 'w'), indent=2)
print(f'output {total:.1f} s, progress factor {factor}×, real {real_minutes:.1f} min after payment')
for s, o in zip(segs, starts):
    print(f'  out {o:6.1f}s  src +{s[0] - pts[0]:6.1f}..{s[1] - pts[0]:6.1f}  {s[2]:5.1f}×  {s[3]}')
