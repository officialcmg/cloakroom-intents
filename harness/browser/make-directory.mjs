#!/usr/bin/env node
// Prints the website's maker directory (web/src/lib/directory.ts) for a maker folder, from
// the signed entry that `zecret directory` prints. Use it to publish the directory of a
// maker on testnet next to web/public/config/testnet.json:
//
//   node harness/browser/make-directory.mjs <maker dir> <USDC per ZEC> testnet > web/public/config/testnet-directory.json
//
// Options by environment: ZECRET_BIN (default crates/target/debug/zecret), MIN_ZEC (0.01),
// MAX_ZEC (2), PREMIUM_MIN_ZEC (premium_zats of the maker config), FEE_ZEC (0.0001).
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const [dir, rate, network = 'testnet'] = process.argv.slice(2);
if (!dir || !rate) {
  console.error('usage: make-directory.mjs <maker dir> <USDC per ZEC> [testnet|regtest|mainnet]');
  process.exit(2);
}
const bin = process.env.ZECRET_BIN || path.join(root, 'crates/target/debug/zecret');
const e = JSON.parse(execFileSync(bin, ['directory', '--dir', dir], { encoding: 'utf8' }));
const cfg = JSON.parse(fs.readFileSync(path.join(dir, 'config.json'), 'utf8'));
const zec = (zats) => (zats / 1e8).toFixed(8).replace(/0+$/, '').replace(/\.$/, '');
const out = {
  format: 'zecret-maker-directory',
  version: 1,
  network,
  updatedAt: new Date().toISOString(),
  status: 'online',
  maker: { name: 'Zecret maker', zcashAddress: e.ua, requestKey: e.rfq_pub, nearAccount: e.near_account, escrowContract: cfg.chains.escrow, nearPublicKey: e.near_pk, epoch: e.epoch, ...(e.limits ? { signedLimits: e.limits } : {}) },
  rate: { usdcPerZec: rate, validForSeconds: 300 },
  limits: { minZec: process.env.MIN_ZEC || '0.01', maxZec: process.env.MAX_ZEC || '2' },
  premium: { bps: 0, minZec: process.env.PREMIUM_MIN_ZEC || zec(cfg.premium_zats) },
  zcashFeePerSendZec: process.env.FEE_ZEC || '0.0001',
  signature: e.sig,
};
console.log(JSON.stringify(out, null, 2));
