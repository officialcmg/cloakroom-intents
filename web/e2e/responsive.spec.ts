// No horizontal scroll on narrow phones, on every main screen and on error states with
// long unbroken strings (keys, account ids, addresses).
import { expect, test, type Page } from '@playwright/test';
import { mockOneClick } from './oneclick-mock';
import { createAndSave, fillForm, nearWithoutMakerKey, open, openDemoControls, pay } from './helpers';

async function noHorizontalScroll(page: Page, what: string) {
  await page.waitForTimeout(150);
  const [scroll, client] = await page.evaluate(() => [document.documentElement.scrollWidth, document.documentElement.clientWidth]);
  expect(scroll, `${what}: page is ${scroll}px wide in a ${client}px viewport`).toBeLessThanOrEqual(client);
}

for (const width of [320, 390]) {
  test.describe(`${width} px wide`, () => {
    test.use({ viewport: { width, height: 740 } });

    test('landing page', async ({ page }) => {
      await mockOneClick(page);
      await page.goto('/');
      await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
      for (const q of await page.locator('.faq-item summary').all()) await q.click();
      await noHorizontalScroll(page, 'landing');
    });

    test('swap form, price details, token picker, banner details', async ({ page }) => {
      await open(page, 'speed=20');
      await noHorizontalScroll(page, 'empty form');
      await fillForm(page, { returnAddress: 'utest13x4neahaknjvdwe5j4h5le2zhfh502nava4jf5g43z9n8ynk9g5whlvsztnn9lm88eq83zzpvsjst0vnfda32lywxv9uz8qgk8wtgr84d6da980fg0wrj2en9hnywv4khl35008f263gztnlqa3zf0he97k0ahttlmmhm7e7ycmn45nm' });
      await page.getByText('How this price is made').click();
      await page.getByTestId('demo-banner').getByRole('button', { name: 'Details' }).click();
      await noHorizontalScroll(page, 'filled form');
      await page.getByRole('button', { name: 'You receive' }).click();
      await page.getByRole('combobox').fill('usdc');
      await noHorizontalScroll(page, 'token picker');
    });

    test('recovery, payment, progress, done', async ({ page }) => {
      await open(page, 'speed=100&pause=committing');
      await fillForm(page);
      await page.getByRole('button', { name: 'Continue' }).click();
      await expect(page.getByRole('heading', { name: 'Save your recovery file' })).toBeVisible();
      await noHorizontalScroll(page, 'recovery');
      const download = page.waitForEvent('download');
      await page.getByRole('button', { name: 'Save recovery file' }).click();
      await download;
      await page.getByRole('button', { name: 'Continue to payment' }).click();
      await expect(page.getByTestId('deposit-address')).toBeVisible();
      await noHorizontalScroll(page, 'payment');
      await pay(page);
      await expect(page.getByTestId('now-title')).toHaveText(/step 1 of 2/, { timeout: 30_000 });
      await noHorizontalScroll(page, 'progress');
      await openDemoControls(page);
      await noHorizontalScroll(page, 'demo controls');
      await page.getByRole('button', { name: 'Skip this wait' }).click();
      await page.keyboard.press('Escape');
      await expect(page.getByTestId('done-title')).toBeVisible({ timeout: 30_000 });
      await page.getByText('All steps and transactions').click();
      await noHorizontalScroll(page, 'done');
    });

    test('refund that asks for an address, and the refunded screen', async ({ page }) => {
      await open(page, 'speed=100&fail=quote-low');
      await fillForm(page);
      await createAndSave(page);
      await pay(page);
      await expect(page.getByTestId('now-title')).toHaveText('Where should your ZEC go?', { timeout: 30_000 });
      await noHorizontalScroll(page, 'refund address');
      await page.getByLabel('Your Zcash address').fill('x'.repeat(300));
      await page.getByRole('button', { name: 'Send my ZEC here' }).click();
      await noHorizontalScroll(page, 'refund address error');
    });

    test('restore with an error', async ({ page }) => {
      await open(page);
      await page.getByRole('button', { name: /Restore a swap/ }).click();
      await page.getByTestId('recovery-input').setInputFiles({ name: 'x.json', mimeType: 'application/json', buffer: Buffer.from('{"hello":1}') });
      await expect(page.getByText('This file cannot be used')).toBeVisible();
      await noHorizontalScroll(page, 'restore error');
    });

    test('error states with long unbroken messages', async ({ page, context }) => {
      await mockOneClick(page);
      // A refused link: the message quotes 80 characters of the link without a break.
      await page.goto(`/swap?config=https://attacker.example/${'a'.repeat(200)}`);
      await expect(page.getByTestId('start-failure')).toBeVisible();
      await noHorizontalScroll(page, 'refused config');

      // The maker check of the hosted site: an ed25519 key and an account id.
      await nearWithoutMakerKey(context);
      await page.goto('/swap?config=testnet');
      await expect(page.getByRole('heading', { name: 'The maker could not be verified' })).toBeVisible({ timeout: 60_000 });
      await page.getByTestId('network-banner').getByRole('button', { name: 'Details' }).click();
      await noHorizontalScroll(page, 'maker check');
      const box = await page.getByTestId('failure-detail').boundingBox();
      expect(box!.x + box!.width).toBeLessThanOrEqual(width);
    });
  });
}
