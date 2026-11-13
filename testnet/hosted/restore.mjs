#!/usr/bin/env node
// Restores a swap on the hosted site from its recovery file in a fresh browser and waits
// until the page reports the leftover ZEC as sent back. Used after a faucet-paid swap.
//   node testnet/hosted/restore.mjs <site URL> <recovery file>
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const { chromium } = createRequire(path.join(ROOT, 'web/package.json'))('playwright');
const [site, file] = process.argv.slice(2);
const t0 = Date.now();
const log = (m) => console.log(`[restore +${Math.round((Date.now() - t0) / 1000)}s] ${m}`);
const browser = await chromium.launch();
const page = await browser.newPage();
await page.goto(new URL('/swap', site).href);
await page.getByRole('button', { name: /Restore a swap/ }).click({ timeout: 180_000 });
await page.getByTestId('recovery-input').setInputFiles(file);
log('recovery file loaded');
const notice = page.getByText(/ZEC was left over/);
await notice.waitFor({ timeout: 20 * 60_000 });
const box = await notice.locator('..').innerText();
log(box.replace(/\s+/g, ' '));
await page.getByText(/It went back to/).waitFor({ timeout: 20 * 60_000 });
log('the page says the leftover went back');
await page.waitForTimeout(90_000); // let the engine see the send mined
await browser.close();
