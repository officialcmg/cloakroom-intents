// Takes screenshots of the key screens in both themes, on a desktop and a phone viewport.
// Run: npm run screenshots. Output: web/screenshots/<name>-<theme>-<device>.png
import { expect, test, type Page } from '@playwright/test';
import { createAndSave, fillForm, nearWithoutMakerKey, open, openDemoControls, pay, RETURN_ADDRESS } from './helpers';
import { mockOneClick } from './oneclick-mock';

const DIR = 'screenshots';
const PHONE = { width: 412, height: 915 };
const DESKTOP = { width: 1280, height: 900 };

async function shot(page: Page, name: string, opts: { phone?: boolean; full?: boolean } = {}) {
  const sizes: [string, { width: number; height: number }][] = [['desktop', DESKTOP]];
  if (opts.phone !== false) sizes.push(['phone', PHONE]);
  for (const [device, size] of sizes) {
    await page.setViewportSize(size);
    for (const scheme of ['light', 'dark'] as const) {
      await page.emulateMedia({ colorScheme: scheme, reducedMotion: 'reduce' });
      await page.evaluate(() => window.scrollTo(0, 0));
      await page.waitForTimeout(150);
      await page.screenshot({ path: `${DIR}/${name}-${scheme}-${device}.png`, fullPage: opts.full ?? true, animations: 'disabled' });
    }
  }
  await page.setViewportSize(DESKTOP);
}

// The landing page and the swap form at 390 and 1440 px, at twice the pixel density, full
// page: screenshots/<landing|swap-form>-<theme>-<width>@2x.png
for (const scheme of ['light', 'dark'] as const) {
  test.describe(`high density, ${scheme}`, () => {
    test.use({ deviceScaleFactor: 2, colorScheme: scheme, reducedMotion: 'reduce' });
    for (const width of [390, 1440]) {
      test(`landing page and swap form, ${width} px`, async ({ page }) => {
        await page.setViewportSize({ width, height: 900 });
        await mockOneClick(page);
        await page.goto('/');
        await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
        await page.evaluate(() => document.fonts.ready);
        await page.screenshot({ path: `${DIR}/landing-${scheme}-${width}@2x.png`, fullPage: true, animations: 'disabled' });
        await open(page);
        await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
        await expect(page.locator('.token-select-text')).toContainText('USDC');
        await page.evaluate(() => document.fonts.ready);
        await page.screenshot({ path: `${DIR}/swap-form-${scheme}-${width}@2x.png`, fullPage: true, animations: 'disabled' });
      });
    }
  });
}

async function skip(page: Page) {
  await openDemoControls(page);
  await page.getByRole('button', { name: 'Skip this wait' }).click();
  await page.keyboard.press('Escape');
}

test.describe.configure({ mode: 'parallel' });

test('landing page', async ({ page }) => {
  await mockOneClick(page);
  await page.goto('/');
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
  await shot(page, '00-landing');
});

test('maker check failed (testnet)', async ({ page, context }) => {
  await nearWithoutMakerKey(context);
  await mockOneClick(page);
  await page.goto('/swap?config=testnet');
  await expect(page.getByRole('heading', { name: 'The maker could not be verified' })).toBeVisible({ timeout: 60_000 });
  await shot(page, '18-maker-check-failed');
});

test('form, price and token picker', async ({ page }) => {
  await open(page);
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
  await shot(page, '01-form-empty');
  await fillForm(page, { returnAddress: RETURN_ADDRESS });
  await page.getByText('How this price is made').click();
  await shot(page, '02-form-price');
  await page.getByRole('button', { name: 'You receive' }).click();
  await page.getByRole('combobox').fill('sol');
  await shot(page, '03-token-picker', { full: false });
});

test('recovery file and payment', async ({ page }) => {
  await open(page, 'speed=1');
  await fillForm(page);
  await page.getByRole('button', { name: 'Continue' }).click();
  await expect(page.getByRole('heading', { name: 'Save your recovery file' })).toBeVisible();
  await shot(page, '04-recovery');
  const download = page.waitForEvent('download');
  await page.getByRole('button', { name: 'Save recovery file' }).click();
  await download;
  await page.getByRole('button', { name: 'Continue to payment' }).click();
  await expect(page.getByRole('heading', { name: /Send exactly/ })).toBeVisible();
  await shot(page, '05-pay');
  await page.getByRole('button', { name: 'Pay only 40%' }).click();
  await expect(page.getByRole('heading', { name: /Send 0.60 ZEC more/ })).toBeVisible();
  await shot(page, '06-pay-partial', { phone: false });
});

test('progress while the browser proves', async ({ page }) => {
  await open(page, 'speed=1');
  await fillForm(page);
  await createAndSave(page);
  await pay(page);
  await expect(page.getByTestId('now-title')).toHaveText('Payment seen', { timeout: 20_000 });
  await skip(page);
  await expect(page.getByTestId('now-title')).toHaveText('Writing to the maker');
  await page.waitForTimeout(4000);
  await shot(page, '07-progress-proving');
});

test('progress while Zcash confirms', async ({ page }) => {
  await open(page, 'speed=50&pause=funding_confirming');
  await fillForm(page);
  await createAndSave(page);
  await pay(page);
  await expect(page.getByText('Zcash confirmations: 1 of 2')).toBeVisible({ timeout: 30_000 });
  await shot(page, '08-progress-confirmations');
});

test('claim window: keep the tab open', async ({ page }) => {
  await open(page, 'speed=50&pause=committing');
  await fillForm(page);
  await createAndSave(page);
  await pay(page);
  await expect(page.getByTestId('now-title')).toHaveText(/step 1 of 2/, { timeout: 30_000 });
  await shot(page, '09-progress-keep-open');
});

test('done', async ({ page }) => {
  await open(page, 'speed=100');
  await fillForm(page);
  await createAndSave(page);
  await pay(page);
  await expect(page.getByTestId('done-title')).toBeVisible({ timeout: 30_000 });
  await shot(page, '10-done');
});

test('exit price below the minimum', async ({ page }) => {
  await open(page, 'speed=100&fail=exit-price');
  await fillForm(page);
  await createAndSave(page);
  await pay(page);
  await expect(page.getByTestId('now-title')).toHaveText('Delivery paused. Your USDC is safe.', { timeout: 30_000 });
  await shot(page, '11-exit-below-minimum');
});

test('delivery in progress', async ({ page }) => {
  await open(page, 'speed=100&pause=exiting.delivering');
  await fillForm(page);
  await createAndSave(page);
  await pay(page);
  await expect(page.getByTestId('now-title')).toHaveText('Delivering USDC on Base', { timeout: 45_000 });
  await shot(page, '11b-exit-delivering');
});

test('exit failed', async ({ page }) => {
  await open(page, 'speed=100&fail=exit-fail');
  await fillForm(page);
  await createAndSave(page);
  await pay(page);
  await expect(page.getByRole('button', { name: 'Try the delivery again' })).toBeVisible({ timeout: 30_000 });
  await shot(page, '12-exit-failed');
});

test('quote below minimum: refund asks for an address, then refunded', async ({ page }) => {
  await open(page, 'speed=100&fail=quote-low');
  await fillForm(page);
  await createAndSave(page);
  await pay(page);
  await expect(page.getByTestId('now-title')).toHaveText('Where should your ZEC go?', { timeout: 30_000 });
  await shot(page, '13-refund-needs-address');
  await page.getByLabel('Your Zcash address').fill(RETURN_ADDRESS);
  await page.getByRole('button', { name: 'Send my ZEC here' }).click();
  await expect(page.getByTestId('refunded-title')).toBeVisible({ timeout: 30_000 });
  await shot(page, '14-refunded');
});

test('maker went quiet: recovering', async ({ page }) => {
  await open(page, 'speed=100&fail=maker-quiet&pause=recovering');
  await fillForm(page, { returnAddress: RETURN_ADDRESS });
  await createAndSave(page);
  await pay(page);
  await expect(page.getByTestId('now-title')).toHaveText('Taking your ZEC back', { timeout: 40_000 });
  await shot(page, '15-maker-quiet-recovering');
});

test('restore and failure screens', async ({ page }) => {
  await open(page);
  await page.getByRole('button', { name: /Restore a swap/ }).click();
  await expect(page.getByRole('heading', { name: 'Restore a swap' })).toBeVisible();
  await page.getByTestId('recovery-input').setInputFiles({ name: 'x.json', mimeType: 'application/json', buffer: Buffer.from('{"hello":1}') });
  await expect(page.getByText('This file cannot be used')).toBeVisible();
  await shot(page, '16-restore');
  await page.route('**/maker-directory.json', (r) => r.fulfill({ status: 503, body: 'down' }));
  await page.reload();
  await expect(page.getByRole('heading', { name: 'The maker’s details did not load' })).toBeVisible();
  await shot(page, '17-directory-failed', { phone: false });
});
