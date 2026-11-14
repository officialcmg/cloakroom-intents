// Writes src/qr.js: the QR code of the site URL as one SVG path, for the end card.
//   node video/make_qr.mjs
// Error correction Q, quiet zone of 4 modules, one unit per module in the viewBox.
import { createRequire } from 'node:module';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const HERE = path.dirname(fileURLToPath(import.meta.url));
const QRCode = createRequire('/home/user/zecret/web/package.json')('qrcode');

const URL = 'https://cloakroom.chrismg.dev';
const QUIET = 4;
const qr = QRCode.create(URL, { errorCorrectionLevel: 'Q' });
const n = qr.modules.size;
let d = '';
for (let y = 0; y < n; y++) {
  // One horizontal run per row segment keeps the path short.
  let x = 0;
  while (x < n) {
    if (!qr.modules.get(y, x)) { x++; continue; }
    let e = x;
    while (e < n && qr.modules.get(y, e)) e++;
    d += `M${x + QUIET} ${y + QUIET}h${e - x}v1h${x - e}z`;
    x = e;
  }
}
const total = n + 2 * QUIET;
const out = `// Written by make_qr.mjs. The QR code of ${URL}: error correction Q, version ${qr.version},
// ${n} x ${n} modules plus a quiet zone of ${QUIET} modules (viewBox ${total} x ${total}).
window.QR = { url: '${URL}', modules: ${total}, path: '${d}' };
`;
fs.writeFileSync(path.join(HERE, 'src/qr.js'), out);
console.log(`version ${qr.version}, ${n} modules, ${total} with quiet zone`);
