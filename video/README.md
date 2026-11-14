# Launch film

The launch film of Cloakroom Intents: 70 seconds, 1920 x 1080, 30 fps, silent. Every
frame is rendered from HTML, CSS and SVG in headless Chromium, so a frame is the same on
every render. The product scene uses one real swap, recorded on the live testnet site.

Output: `testnet/hosted/demo/cloakroom-intents-launch.mp4` (git-ignored), its poster
`cloakroom-intents-launch-poster.jpg`, and `launch-facts.json` (the swap's transactions
and timings, the speed of each sped-up part, and the source of every claim on screen).

## Files

| File | What it is |
|---|---|
| `src/film.html`, `src/film.css` | The six scenes and their styles. All on-screen text of scenes 1, 2, 3, 5 and 6 is here or in `src/scenes.js`. |
| `src/scenes.js` | The timeline: scene windows (`SC`), and one function per scene that draws it at time t. |
| `src/product.js` | Scene 4: the browser frame, the camera moves, the callouts and the clock. |
| `src/engine.js` | Easing curves and helpers. Nothing reads the clock. |
| `src/brand.js` | The name, URL and logo mark, in one place. |
| `src/chains.js` | The destination chains, copied from `web/src/lib/chainNames.ts`. |
| `src/footage.js` | Written by `cut_footage.py`: the footage frames and the page's boxes. |
| `record.mjs` | Records one real swap on the live site (adapted from `testnet/hosted/record-demo.mjs`). |
| `cut_footage.py` | Cuts the recording into the footage of scene 4. |
| `render.mjs` | Renders frames or stills. |
| `render.sh` | Cuts, renders and encodes the film. |
| `assets/fonts/` | The site's fonts (SIL OFL 1.1; licences next to them). |

## Re-render

Needs Node with the `web/` dependencies (Playwright), Chromium in
`/opt/pw-browsers`, Python 3, ffmpeg, and for a new recording Xvfb and the testnet payer
wallet.

```sh
# 1. Record a real swap (about 10 minutes; pays 0.01 TAZ from the testnet wallet).
#    The raw folder is large: keep it outside the repository.
PLAYWRIGHT_BROWSERS_PATH=/opt/pw-browsers \
  node video/record.mjs https://cloakroom.chrismg.dev .tools/testnet/taker zecret-3f109eea1e.testnet /path/to/raw

# 2. Cut, render and encode (about 10 minutes on 4 cores; frames go to the work folder).
video/render.sh /path/to/raw /path/to/work
```

`DEMO_DRY=1` before step 1 rehearses the recording and stops before the payment.

Stills while you edit: `node video/render.mjs stills /tmp/st 3,12.5,40` writes PNGs of
those film times.

## Change a line of text

- Scenes 1, 2, 5 and 6: edit the text in `src/film.html`. Headings with `data-text` mark
  the serif italic part with `*stars*`.
- Scene 3: the three numbered lines are `BEATS` in `src/scenes.js`; the labels of the
  drawing are in the HTML that follows.
- Scene 5: the four numbers are `STATS` in `src/scenes.js`.
- Scene 4: the callouts are `CALLOUTS` in `src/product.js`.
- Name, URL, logo: `src/brand.js`.

Every claim must have a source: add it to `launch-facts.json` when you change one.

## Swap the footage

Record a new swap (step 1) and run `render.sh` with the new raw folder. The camera and
the callouts follow the page by the boxes that `record.mjs` logs (`boxes.json`), so a new
recording needs no hand-placed coordinates. The speed of the progress part is chosen so
that it lasts about 9.6 seconds of film; the clock always shows the true time since the
payment.
