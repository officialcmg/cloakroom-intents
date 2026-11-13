import { expect, type Page } from '@playwright/test';
import { mockOneClick, type MockOptions } from './oneclick-mock';

export const EVM_ADDRESS = '0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed';
export const SOL_ADDRESS = '9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM';
/** A testnet unified address with an Orchard receiver (made with zcash_address). */
export const RETURN_ADDRESS =
  'utest13x4neahaknjvdwe5j4h5le2zhfh502nava4jf5g43z9n8ynk9g5whlvsztnn9lm88eq83zzpvsjst0vnfda32lywxv9uz8qgk8wtgr84d6da980fg0wrj2en9hnywv4khl35008f263gztnlqa3zf0he97k0ahttlmmhm7e7ycmn45nm';

/** Opens the swap app (/swap) with `query`, with the 1Click API mocked. */
export async function open(page: Page, query = '', mock: MockOptions = {}) {
  const handle = await mockOneClick(page, mock);
  await page.goto(`/swap${query ? `?${query}` : ''}`);
  return handle;
}

/** Opens "Demo controls", which sit under "Details" in the demo banner. */
export async function openDemoControls(page: Page) {
  const banner = page.getByTestId('demo-banner');
  const toggle = banner.getByRole('button', { name: 'Details' });
  if ((await toggle.getAttribute('aria-expanded')) !== 'true') await toggle.click();
  await banner.getByRole('button', { name: 'Demo controls' }).click();
}

/** Fills the form with 1 ZEC to USDC on Base (the default token) and waits for the price. */
export async function fillForm(page: Page, opts: { amount?: string; address?: string; returnAddress?: string } = {}) {
  await page.getByLabel('You pay').fill(opts.amount ?? '1');
  await page.getByLabel(/^Your USDC address on Base/).fill(opts.address ?? EVM_ADDRESS);
  if (opts.returnAddress) {
    await page.getByRole('button', { name: /Add a Zcash return address/ }).click();
    await page.getByLabel(/Zcash return address/).fill(opts.returnAddress);
  }
  await expect(page.getByTestId('price-line')).toContainText('You receive at least');
  await expect(page.locator('.price')).not.toHaveClass(/is-stale/);
}

/** From the filled form to the payment screen, saving the recovery file on the way. */
export async function createAndSave(page: Page): Promise<string> {
  await page.getByRole('button', { name: 'Continue' }).click();
  await expect(page.getByRole('heading', { name: 'Save your recovery file' })).toBeVisible();
  const download = page.waitForEvent('download');
  await page.getByRole('button', { name: 'Save recovery file' }).click();
  const file = await download;
  const path = await file.path();
  await expect(page.getByText('Recovery file saved')).toBeVisible();
  await page.getByRole('button', { name: 'Continue to payment' }).click();
  await expect(page.getByRole('heading', { name: /Send exactly 1.00 ZEC/ })).toBeVisible();
  return path;
}

export async function pay(page: Page) {
  await page.getByRole('button', { name: 'Simulate the payment' }).click();
}

export async function nowTitle(page: Page) {
  return page.getByTestId('now-title');
}
