// Writes src/chainIcons.js from the site's chain logos (web/src/ui/chainIconData.ts), so
// the film page can load them without a build step. The logos and their licences are
// listed in web/src/assets/chains/SOURCES.md.
//
//   node video/make_chain_icons.mjs
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const SRC = path.join(HERE, '../web/src/ui/chainIconData.ts');
const OUT = path.join(HERE, 'src/chainIcons.js');
const ts = fs.readFileSync(SRC, 'utf8');

// Each entry: "Name": { viewBox: "...", body: "..." },  (the strings are JSON strings)
const re = /^\s*("(?:[^"\\]|\\.)*"):\s*\{\s*viewBox:\s*("(?:[^"\\]|\\.)*"),\s*body:\s*("(?:[^"\\]|\\.)*"),?\s*\},?/gm;
const icons = {};
for (const m of ts.matchAll(re)) icons[JSON.parse(m[1])] = { viewBox: JSON.parse(m[2]), body: JSON.parse(m[3]) };

const names = JSON.parse(fs.readFileSync(path.join(HERE, 'src/chains.js'), 'utf8').match(/CHAIN_NAMES = (\[.*\]);/)[1]);
const missing = names.filter((n) => !icons[n]);
if (missing.length) throw new Error('no logo for: ' + missing.join(', '));

fs.writeFileSync(OUT, '// Written by make_chain_icons.mjs from web/src/ui/chainIconData.ts. Do not edit.\n' +
  '// Logos and licences: web/src/assets/chains/SOURCES.md.\n' +
  'window.CHAIN_ICONS = ' + JSON.stringify(icons) + ';\n');
console.log(`wrote ${OUT}: ${Object.keys(icons).length} logos`);
