import { createRequire } from 'node:module';
import fs from 'node:fs';
const require = createRequire('/home/user/zecret/web/package.json');
const { chromium } = require('playwright');

const OUT = '/home/user/zecret/video/logo-options';
const FONTS = '/home/user/zecret/web/src/assets/fonts';
const G = { g950: '#072b23', g900: '#0a3a2f', g800: '#0d4f3e', g600: '#17735a', g500: '#2a8f72', mint: '#9fe3c6', tint: '#f5faf7', tint2: '#eaf3ee' };

// Each glyph draws in a 32x32 box, centred on 16,16. `c` is the glyph colour, `id` a unique prefix.
// Knockouts use fill-rule evenodd or a mask, so the tile colour shows through on any background.
const options = [
  {
    name: 'Claim Tag',
    idea: 'A coat-check tag with its punched hole and two notches. You hand over the coat and keep the tag: the claim.',
    glyph: (c) => `<path transform="rotate(-90 16 16) translate(0 -0.1)" fill="${c}" stroke="${c}" stroke-width="1.2" stroke-linejoin="round" fill-rule="evenodd" d="M16 6.6 22.4 11.9V16.2A1.8 1.8 0 0 0 22.4 19.8V23.6A1.6 1.6 0 0 1 20.8 25.2H11.2A1.6 1.6 0 0 1 9.6 23.6V19.8A1.8 1.8 0 0 0 9.6 16.2V11.9ZM16 9.7A2.4 2.4 0 1 0 16 14.5 2.4 2.4 0 1 0 16 9.7Z"/>`,
  },
  {
    name: 'Torn Stub',
    idea: 'A cloakroom ticket torn into ticket and stub. Two halves of one claim: the protocol splits the claim key the same way.',
    glyph: (c) => `<path fill="${c}" fill-rule="evenodd" d="M8.4 10H19.2V22H8.4A1.6 1.6 0 0 1 6.8 20.4V18.2A2.2 2.2 0 0 0 6.8 13.8V11.6A1.6 1.6 0 0 1 8.4 10Z"/><path fill="${c}" d="M21.2 8.6H23.6A1.6 1.6 0 0 1 25.2 10.2V19A1.6 1.6 0 0 1 23.6 20.6H21.2Z"/>`,
  },
  {
    name: 'Quiet Hood',
    idea: 'A hood reduced to one pointed arch with an empty face. The cloak in the name: the person is there, the identity is not.',
    glyph: (c) => `<path fill="${c}" fill-rule="evenodd" d="M16 5.8C20.9 8.4 24.2 13 24.2 18.6V24A1.6 1.6 0 0 1 22.6 25.6H9.4A1.6 1.6 0 0 1 7.8 24V18.6C7.8 13 11.1 8.4 16 5.8ZM16 14.2A3.9 3.9 0 1 0 16 22 3.9 3.9 0 1 0 16 14.2Z"/>`,
  },
  {
    name: 'Closed C',
    idea: 'A heavy C around a single point: the C of Cloakroom, and a coin held inside a closed hand. Reads as "Ci" at a glance.',
    glyph: (c) => `<g transform="translate(0.7 0)"><path d="M21.6 10.6A7.8 7.8 0 1 0 21.6 21.4" fill="none" stroke="${c}" stroke-width="3.4" stroke-linecap="round"/><circle cx="15.6" cy="16" r="2.7" fill="${c}"/></g>`,
  },
  {
    name: 'Shield Routes',
    idea: 'A shield on its side, its point opening into three routes that end at three chains. One shielded source, many places to land.',
    glyph: (c) => `<path fill="${c}" stroke="${c}" stroke-width="1.4" stroke-linejoin="round" d="M6.6 10.2V21.8H10.6C13.2 21.8 15 19.6 16.2 16 15 12.4 13.2 10.2 10.6 10.2Z"/><path d="M17.4 16H23.2M17.4 16 22.6 10M17.4 16 22.6 22" fill="none" stroke="${c}" stroke-width="2" stroke-linecap="round"/><circle cx="23.6" cy="9.4" r="2.1" fill="${c}"/><circle cx="24.4" cy="16" r="2.1" fill="${c}"/><circle cx="23.6" cy="22.6" r="2.1" fill="${c}"/>`,
  },
  {
    name: 'Drawn Curtain',
    idea: 'A doorway whose curtain is drawn open and tied back. The cloakroom counter, and a view that only opens when you choose.',
    glyph: (c) => `<path fill="${c}" fill-rule="evenodd" d="M8 24.2V15A8 8 0 0 1 24 15V24.2A1.4 1.4 0 0 1 22.6 25.6H9.4A1.4 1.4 0 0 1 8 24.2ZM16 11.4C13.9 11.4 11.7 15.6 10.9 19.3 12.4 20.9 13 23.4 13 25.6H19C19 23.4 19.6 20.9 21.1 19.3 20.3 15.6 18.1 11.4 16 11.4Z"/>`,
  },
  {
    name: 'Wrapped Coin',
    idea: 'A coin with a band wrapped across it. The coin is still there and still whole; it is just covered while it moves.',
    glyph: (c, id) => `<mask id="${id}m"><rect width="32" height="32" fill="#fff"/><g transform="rotate(-38 16 16)"><rect x="0" y="12.1" width="32" height="1.6" fill="#000"/><rect x="0" y="18.3" width="32" height="1.6" fill="#000"/></g></mask><circle mask="url(#${id}m)" cx="16" cy="16" r="9.2" fill="${c}"/>`,
  },
  {
    name: 'Open Cape',
    idea: 'A cape seen from the front, held by one clasp and open at the hem. The cloak, without the hanger.',
    glyph: (c, id) => `<mask id="${id}m"><rect width="32" height="32" fill="#fff"/><circle cx="16" cy="10.6" r="1.7" fill="#000"/><path d="M16 15.2 18.6 26H13.4Z" fill="#000"/></mask><path mask="url(#${id}m)" fill="${c}" stroke="${c}" stroke-width="1.2" stroke-linejoin="round" d="M12.8 7.2H19.2C19.6 13 21.8 19 24.8 24.8 22 25.4 19 25.6 16 25.6S10 25.4 7.2 24.8C10.2 19 12.4 13 12.8 7.2Z"/>`,
  },
  {
    name: 'Split Seal',
    idea: 'A round seal with a keyhole, cut in two and slightly offset. Line the halves up and the keyhole opens: the claim.',
    glyph: (c, id) => `<mask id="${id}k"><rect width="32" height="32" fill="#fff"/><circle cx="16" cy="14.4" r="2.5" fill="#000"/><path d="M14.6 15.6H17.4L18.3 21.2H13.7Z" fill="#000"/></mask><clipPath id="${id}l"><rect x="0" y="0" width="15.15" height="32"/></clipPath><clipPath id="${id}r"><rect x="16.85" y="0" width="16" height="32"/></clipPath><g mask="url(#${id}k)"><g clip-path="url(#${id}l)"><circle cx="16" cy="15.1" r="9" fill="${c}"/></g><g clip-path="url(#${id}r)"><circle cx="16" cy="16.9" r="9" fill="${c}"/></g></g>`,
  },
  {
    name: 'Domino Mask',
    idea: 'A plain eye mask in two curves. Private by default, the same way the shielded pool is.',
    glyph: (c) => `<path fill="${c}" fill-rule="evenodd" d="M5.6 13.4C5.6 10.6 8.6 10 11.6 10.6 13.8 11 15 12 16 12S18.2 11 20.4 10.6C23.4 10 26.4 10.6 26.4 13.4 26.4 18.4 23.8 21.6 20.6 21.6 18.6 21.6 17.6 19.6 16 19.6S13.4 21.6 11.4 21.6C8.2 21.6 5.6 18.4 5.6 13.4ZM11.4 13.4C9.6 13.4 8.6 14.4 8.6 15.4 9.4 16.6 10.6 17.2 11.8 17.2 12.9 17.2 13.6 16.4 13.6 15.4 13.6 14.2 12.6 13.4 11.4 13.4ZM20.6 13.4C22.4 13.4 23.4 14.4 23.4 15.4 22.6 16.6 21.4 17.2 20.2 17.2 19.1 17.2 18.4 16.4 18.4 15.4 18.4 14.2 19.4 13.4 20.6 13.4Z"/>`,
  },
];

let n = 0;
const uid = () => `u${++n}`;
// tile: 'green' = green tile, white glyph. 'light' = white tile, green glyph. 'bare' = glyph only.
function mark(opt, size, tile = 'green', glyphColor) {
  const id = uid();
  let bg = '';
  let c = glyphColor ?? '#fff';
  if (tile === 'green') {
    bg = `<defs><linearGradient id="${id}g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="${G.g500}"/><stop offset="1" stop-color="${G.g800}"/></linearGradient></defs><rect width="32" height="32" rx="9.6" fill="url(#${id}g)"/><rect x="0.5" y="0.5" width="31" height="31" rx="9.1" fill="none" stroke="#fff" stroke-opacity="0.18"/>`;
  } else if (tile === 'light') {
    bg = `<rect width="32" height="32" rx="9.6" fill="#fff"/>`;
    c = glyphColor ?? G.g800;
  }
  return `<svg width="${size}" height="${size}" viewBox="0 0 32 32" xmlns="http://www.w3.org/2000/svg">${bg}${opt.glyph(c, id)}</svg>`;
}

// Standalone SVG files: the green tile with the white mark, ready for the Logo component.
options.forEach((o, i) => {
  const svg = mark(o, 32, 'green').replace(/ width="32" height="32"/, '').replace(/u\d+/g, 'm');
  fs.writeFileSync(`${OUT}/option-${String(i + 1).padStart(2, '0')}.svg`, svg + '\n');
});

const css = `
@font-face{font-family:'Hanken Grotesk';src:url('file://${FONTS}/HankenGrotesk-latin-wght.woff2') format('woff2');font-weight:100 900}
@font-face{font-family:'Instrument Serif';src:url('file://${FONTS}/InstrumentSerif-latin-400-italic.woff2') format('woff2');font-style:italic}
@font-face{font-family:'Inter';src:url('file://${FONTS}/Inter-latin-wght.woff2') format('woff2');font-weight:100 900}
@font-face{font-family:'Geist Mono';src:url('file://${FONTS}/GeistMono-latin-500.woff2') format('woff2');font-weight:400 500}
*{box-sizing:border-box;margin:0}
body{width:2400px;background:${G.tint};font-family:Inter;color:#0d1f19;padding:96px 96px 72px}
.top{display:flex;justify-content:space-between;align-items:flex-end;margin-bottom:64px}
.eyebrow{font-family:'Geist Mono';font-size:15px;letter-spacing:.12em;text-transform:uppercase;color:${G.g600}}
h1{font-family:'Hanken Grotesk';font-weight:600;font-size:64px;letter-spacing:-.02em;margin-top:14px}
h1 em{font-family:'Instrument Serif';font-weight:400;font-size:1.12em}
.note{max-width:620px;font-size:17px;line-height:1.55;color:#4a5f57}
.card{display:grid;grid-template-columns:360px 220px 220px 220px 1fr 300px;gap:24px;align-items:stretch;background:#fff;border:1px solid #dfe9e4;border-radius:28px;padding:28px;margin-bottom:28px}
.info{padding:8px 8px 8px 4px;display:flex;flex-direction:column}
.num{font-family:'Geist Mono';font-size:15px;color:${G.g600};letter-spacing:.08em}
.name{font-family:'Hanken Grotesk';font-weight:600;font-size:30px;letter-spacing:-.01em;margin:8px 0 12px}
.idea{font-size:15px;line-height:1.5;color:#4a5f57}
.cell{border-radius:20px;display:flex;align-items:center;justify-content:center;height:244px}
.w{background:#fff;border:1px solid #e6efea}.d{background:${G.g950}}.m{background:${G.tint2}}
.lock{display:flex;flex-direction:column;gap:16px}
.lock .cell{height:114px;justify-content:flex-start;padding-left:36px}
.logo{display:inline-flex;align-items:center;gap:10px}
.word{font-family:'Hanken Grotesk';font-weight:600;font-size:19px;letter-spacing:.01em;line-height:1;white-space:nowrap}
.word em{font-family:'Instrument Serif';font-weight:400;font-size:1.12em;letter-spacing:0}
.d .word{color:#fff}
.small{display:grid;grid-template-columns:1fr 1fr;gap:16px}
.small .cell{height:114px;flex-direction:column;gap:12px}
.lab{font-family:'Geist Mono';font-size:12px;color:#7a8f87;letter-spacing:.06em}
.d .lab{color:#8fb5a6}
.row{display:flex;gap:14px;align-items:center}
.capt{display:flex;gap:24px;font-family:'Geist Mono';font-size:12px;color:#7a8f87;letter-spacing:.06em;margin:0 0 10px 0}
`;

function card(o, i) {
  const num = String(i + 1).padStart(2, '0');
  const lock = (cls) => `<div class="cell ${cls}"><span class="logo">${mark(o, 30)}<span class="word">Cloakroom<em> Intents</em></span></span></div>`;
  const small = (cls) => `<div class="cell ${cls}"><div class="row">${mark(o, 32)}${mark(o, 16)}</div><div class="lab">32 · 16 px</div></div>`;
  return `<section class="card">
  <div class="info"><div class="num">OPTION ${num}</div><div class="name">${o.name}</div><div class="idea">${o.idea}</div></div>
  <div class="cell w">${mark(o, 168)}</div>
  <div class="cell d">${mark(o, 168)}</div>
  <div class="cell m">${mark(o, 168, 'light')}</div>
  <div class="lock">${lock('w')}${lock('d')}</div>
  <div class="small">${small('w')}${small('d')}<div class="cell w">${mark(o, 32, 'bare', G.g800)}${''}</div><div class="cell d">${mark(o, 32, 'bare', '#fff')}</div></div>
</section>`;
}

const html = (list, start, label) => `<!doctype html><html><head><meta charset="utf-8"><style>${css}
.small{grid-template-rows:1fr 1fr}.small .cell:nth-child(n+3){display:none}</style></head><body>
<div class="top"><div><div class="eyebrow">Logo mark options · ${label}</div><h1>Cloakroom<em> Intents</em></h1></div>
<div class="note">Ten marks for the rounded-square tile used in the header and favicon. Each row: the mark on white, on deep green, reversed on a light tile, in the header lock-up, and at the two favicon sizes at true pixel size.</div></div>
${list.map((o, k) => card(o, start + k)).join('\n')}
</body></html>`;

fs.writeFileSync(`${OUT}/README.md`, '# Logo mark options\n\nTen marks for the rounded-square tile (32x32 viewBox, white mark on the green tile). `board.png` shows each one on white, on deep green, reversed, in the header lock-up and at 32 px and 16 px.\n\n' + options.map((o, i) => `${i + 1}. **${o.name}** (\`option-${String(i + 1).padStart(2, '0')}.svg\`): ${o.idea}`).join('\n') + '\n');
const scratch = process.argv[2];
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 2400, height: 1000 }, deviceScaleFactor: 1 });
const sheet = `<!doctype html><body style="margin:0;padding:8px;background:#fff">` + options.map((o)=>`<div style="display:flex;gap:6px;align-items:center;margin:4px">${mark(o,32)}${mark(o,16)}<span style="display:inline-flex;gap:6px;background:${G.g950};padding:4px">${mark(o,32)}${mark(o,16)}</span>${mark(o,16,'light')}</div>`).join('') + '</body>';
fs.writeFileSync(`${scratch}/sheet.html`, sheet);
await page.setViewportSize({ width: 260, height: 420 });
await page.goto(`file://${scratch}/sheet.html`);
await page.screenshot({ path: `${scratch}/sheet.png`, fullPage: true });
await page.setViewportSize({ width: 2400, height: 1000 });
const parts = [[options, 0, 'Board']];
for (const [list, start, label] of parts) {
  fs.writeFileSync(`${scratch}/board.html`, html(list, start, label));
  await page.goto(`file://${scratch}/board.html`);
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(300);
  await page.screenshot({ path: `${OUT}/board.png`, fullPage: true });
  // Zoomed crops of the 16 px cells for checking.
  const h = await page.evaluate(() => document.body.scrollHeight);
  console.log('height', h);
}
await browser.close();
