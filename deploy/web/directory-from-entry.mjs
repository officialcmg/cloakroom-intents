#!/usr/bin/env node
// Prints the website's maker directory (web/src/lib/directory.ts) from a signed entry that
// `zecret directory` printed, for a maker whose folder is not on this computer (the hosted
// maker: its start script logs the entry as `directory entry: {...}`, docs/HOSTING.md).
//
//   node deploy/web/directory-from-entry.mjs <entry.json> <USDC per ZEC> [escrow] > web/public/config/testnet-directory.json
//   node deploy/web/directory-from-entry.mjs <entry.json> <USDC per ZEC> <escrow> > web/public/config/mainnet-directory.json
//
// The network follows the entry's Zcash address: utest1... is testnet, u1... is mainnet.
// On mainnet the escrow has no default: give the deployed escrow account.
// The limits and the premium shown on the page are the signed limits of the entry. The
// rate is what the page shows before the quote; set it to the maker's price_per_zec
// divided by 10^6.
import fs from 'node:fs';

const [file, rate, escrowArg] = process.argv.slice(2);
if (!file || !rate) {
  console.error('usage: directory-from-entry.mjs <entry.json> <USDC per ZEC> [escrow]');
  process.exit(2);
}
const e = JSON.parse(fs.readFileSync(file, 'utf8'));
for (const k of ['ua', 'rfq_pub', 'near_account', 'near_pk', 'epoch', 'sig', 'limits']) {
  if (e[k] === undefined) throw new Error(`the entry has no ${k}`);
}
const network = e.ua.startsWith('utest1') ? 'testnet' : e.ua.startsWith('u1') ? 'mainnet' : undefined;
if (!network) throw new Error('the entry is not for Zcash testnet or mainnet');
if (!/^\d+(\.\d{1,6})?$/.test(rate) || Number(rate) <= 0) throw new Error('the rate must be a positive USDC amount with at most 6 decimals');
const escrow = escrowArg ?? (network === 'testnet' ? 'escrow2.zecret-3f109eea1e.testnet' : undefined);
if (!escrow) throw new Error('give the mainnet escrow account as the third argument');
if (network === 'mainnet' && (escrow.endsWith('.testnet') || e.near_account.endsWith('.testnet'))) throw new Error('a mainnet entry cannot name .testnet accounts');
const zec = (zats) => (Number(zats) / 1e8).toFixed(8).replace(/0+$/, '').replace(/\.$/, '');
const out = {
  format: 'zecret-maker-directory',
  version: 1,
  network,
  updatedAt: new Date().toISOString(),
  status: 'online',
  maker: { name: network === 'testnet' ? 'Zecret testnet maker' : 'Zecret maker', zcashAddress: e.ua, requestKey: e.rfq_pub, nearAccount: e.near_account, escrowContract: escrow, nearPublicKey: e.near_pk, epoch: e.epoch, signedLimits: e.limits },
  rate: { usdcPerZec: rate, validForSeconds: 300 },
  limits: { minZec: zec(e.limits.min_zats), maxZec: zec(e.limits.max_zats) },
  premium: { bps: 0, minZec: zec(e.limits.premium_zats) },
  zcashFeePerSendZec: '0.0001',
  signature: e.sig,
};
console.log(JSON.stringify(out, null, 2));
