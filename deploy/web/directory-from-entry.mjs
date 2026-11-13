#!/usr/bin/env node
// Prints the website's maker directory (web/src/lib/directory.ts) from a signed entry that
// `zecret directory` printed, for a maker whose folder is not on this computer (the hosted
// maker: its start script logs the entry as `directory entry: {...}`, docs/HOSTING.md).
//
//   node deploy/web/directory-from-entry.mjs <entry.json> <USDC per ZEC> [escrow] > web/public/config/testnet-directory.json
//
// The limits and the premium shown on the page are the signed limits of the entry.
import fs from 'node:fs';

const [file, rate, escrow = 'escrow2.zecret-3f109eea1e.testnet'] = process.argv.slice(2);
if (!file || !rate) {
  console.error('usage: directory-from-entry.mjs <entry.json> <USDC per ZEC> [escrow]');
  process.exit(2);
}
const e = JSON.parse(fs.readFileSync(file, 'utf8'));
for (const k of ['ua', 'rfq_pub', 'near_account', 'near_pk', 'epoch', 'sig', 'limits']) {
  if (e[k] === undefined) throw new Error(`the entry has no ${k}`);
}
if (!e.ua.startsWith('utest1')) throw new Error('the entry is not for Zcash testnet');
const zec = (zats) => (Number(zats) / 1e8).toFixed(8).replace(/0+$/, '').replace(/\.$/, '');
const out = {
  format: 'zecret-maker-directory',
  version: 1,
  network: 'testnet',
  updatedAt: new Date().toISOString(),
  status: 'online',
  maker: { name: 'Zecret testnet maker', zcashAddress: e.ua, requestKey: e.rfq_pub, nearAccount: e.near_account, escrowContract: escrow, nearPublicKey: e.near_pk, epoch: e.epoch, signedLimits: e.limits },
  rate: { usdcPerZec: rate, validForSeconds: 300 },
  limits: { minZec: zec(e.limits.min_zats), maxZec: zec(e.limits.max_zats) },
  premium: { bps: 0, minZec: zec(e.limits.premium_zats) },
  zcashFeePerSendZec: '0.0001',
  signature: e.sig,
};
console.log(JSON.stringify(out, null, 2));
