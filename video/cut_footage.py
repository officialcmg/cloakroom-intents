#!/usr/bin/env python3
"""Turns one recording made by record.mjs into the footage of the film's product scene.

    python3 video/cut_footage.py <raw dir> <frames dir>

Reads <raw dir>/raw.mkv, marks.json, facts.json and boxes.json. Writes one JPEG per film
frame of the product scene to <frames dir> (large: keep it outside the repository) and
video/src/footage.js, which the film loads: per frame the image, the true time since the
payment, the speed, and where the parts of the page were at that moment.

Edit: the form plays sped up (the typing), the price and the payment screen play at real
speed, the wallet's work on the payment is squeezed, and everything from the payment to
the end of the swap plays at one speed factor, so the true time is easy to read off the
clock in the film.
"""
import json
import subprocess
import sys
from pathlib import Path

RAW, FR = Path(sys.argv[1]), Path(sys.argv[2])
HERE = Path(__file__).resolve().parent
FR.mkdir(parents=True, exist_ok=True)
for f in FR.glob('*.jpg'):
    f.unlink()
FPS = 30
PROGRESS_SECS = 9.6  # film seconds for the time from payment to the end of the swap

marks = json.load(open(RAW / 'marks.json'))
facts = json.load(open(RAW / 'facts.json'))
boxes = json.load(open(RAW / 'boxes.json'))


def m(name):
    for x in marks:
        if x['name'] == name:
            return x['t']
    raise KeyError(name)


start = float(subprocess.check_output(['ffprobe', '-v', 'error', '-show_entries', 'format=start_time', '-of', 'csv=p=0', str(RAW / 'raw.mkv')], text=True))
pay_sent = m('pay_sent')
final = m('final_screen')
progress_factor = round((final - pay_sent) / PROGRESS_SECS)

# (source from, source to, speed, kind)
segs = [
    (m('form_ready') + 0.2, m('paste_return') + 0.5, 4.0, 'form'),
    (m('price_wait_end') - 0.3, m('price_shown') + 1.6, 1.0, 'price'),
    (m('pay_screen') - 0.2, m('pay_start') + 0.6, 1.0, 'pay'),
    (m('pay_start') + 0.6, pay_sent, max(1.0, (pay_sent - m('pay_start') - 0.6) / 1.2), 'paying'),
    (pay_sent, final, float(progress_factor), 'progress'),
    (final, final + 5.2, 1.0, 'done'),
]

frames, events, k = [], {}, 0
for a, b, speed, kind in segs:
    n = max(1, round((b - a) / speed * FPS))
    tmp = FR / f'seg_{kind}'
    tmp.mkdir(exist_ok=True)
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-ss', f'{a - start:.3f}', '-i', str(RAW / 'raw.mkv'), '-t', f'{b - a + 0.5:.3f}',
                    '-vf', f'fps={FPS / speed:.6f}', '-frames:v', str(n), '-q:v', '3', str(tmp / '%05d.jpg')], check=True)
    got = sorted(tmp.glob('*.jpg'))
    events[kind] = k / FPS
    for i in range(n):
        src = got[min(i, len(got) - 1)]
        dst = FR / f'{k:05d}.jpg'
        if i < len(got):
            src.rename(dst)
        else:  # the decoder gave a frame less: repeat the last
            dst.write_bytes((FR / f'{k - 1:05d}.jpg').read_bytes())
        rt = a + i * speed / FPS
        bx = None
        for e in boxes:
            if e['t'] <= rt:
                bx = e
            else:
                break
        frames.append({'i': k, 'e': round(rt - pay_sent, 2), 'spd': speed, 'kind': kind, 'b': boxes.index(bx) if bx else 0})
        k += 1
    for f in tmp.glob('*.jpg'):
        f.unlink()
    tmp.rmdir()
events['end'] = k / FPS

# The film's times of stage changes, for the callouts.
def film_t(src):
    o = 0.0
    for a, b, speed, kind in segs:
        if a <= src <= b:
            return o + (src - a) / speed
        o += (b - a) / speed
    return None

stage_t = {x['name'][6:]: film_t(x['t']) for x in marks if x['name'].startswith('stage:')}
real_secs = (m('stage:' + facts.get('finalStage', 'done')) if any(x['name'] == 'stage:' + str(facts.get('finalStage')) for x in marks) else final) - pay_sent
host = facts['site'].replace('https://', '').rstrip('/')
data = {
    'dir': 'file://' + str(FR.resolve()) + '/',
    'host': host,
    'frames': frames,
    'boxes': [{k2: v for k2, v in e.items() if k2 != 't'} for e in boxes],
    'events': events,
    'stageT': stage_t,
    'realSecs': round(real_secs, 1),
    'progressFactor': progress_factor,
    'doneText': facts.get('doneTitle', ''),
}
(HERE / 'src' / 'footage.js').write_text('// Written by cut_footage.py. Do not edit.\nwindow.FOOTAGE = ' + json.dumps(data) + ';\n')
json.dump({'segments': [{'sourceFromSecs': round(a - start, 2), 'sourceToSecs': round(b - start, 2), 'speed': round(s, 2), 'kind': kd} for a, b, s, kd in segs],
           'progressFactor': progress_factor, 'paymentToFinalSecs': round(real_secs, 1), 'filmEvents': events, 'stageFilmTimes': stage_t},
          open(RAW / 'cut.json', 'w'), indent=2)
print(f'{k} frames ({k / FPS:.1f} s), progress {progress_factor}x, real {real_secs:.0f} s after payment')
for a, b, s, kd in segs:
    print(f'  {kd:9s} src +{a - start:7.1f}..{b - start:7.1f}  {s:6.1f}x  -> {(b - a) / s:5.1f} s')
