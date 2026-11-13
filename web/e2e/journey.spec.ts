// The whole journey and every failure case, driven through the page against the
// simulated engine. Runs on a desktop and a phone viewport (see playwright.config.ts).
import { readFileSync } from 'node:fs';
import { expect, test, type Page } from '@playwright/test';
import { mockOneClick } from './oneclick-mock';
import { createAndSave, EVM_ADDRESS, fillForm, open, pay, RETURN_ADDRESS, SOL_ADDRESS } from './helpers';

const now = (page: Page) => page.getByTestId('now-title');

async function demoControls(page: Page, action: string) {
  await page.getByRole('button', { name: 'Demo controls' }).click();
  await page.getByRole('button', { name: action }).click();
}

test('full swap: price, recovery file, payment, progress, delivery', async ({ page }) => {
  const mock = await open(page, 'speed=100');
  await fillForm(page);
  const line = page.getByTestId('price-line');
  await expect(line).toContainText('You pay 1.00 ZEC.');
  await expect(line).toContainText(/You receive at least [\d,.]+ USDC on Base\./);
  // The last quote on the form used the user's own address.
  expect(mock.quotes.at(-1)).toMatchObject({ recipient: EVM_ADDRESS, dry: true, amount: '937838000' });

  const filePath = await createAndSave(page);
  const file = JSON.parse(readFileSync(filePath, 'utf8'));
  expect(file).toMatchObject({ format: 'zecret-recovery', version: 1, network: 'testnet' });
  expect(file.secrets.keyHalf).toMatch(/^[0-9a-f]{64}$/);

  await expect(page.getByTestId('deposit-address')).toHaveText(/^utest1/);
  await expect(page.getByRole('img', { name: /Payment request: 1 ZEC/ })).toBeVisible();
  await pay(page);
  await expect(now(page)).toBeVisible();
  await expect(page.getByTestId('safety')).toContainText('You can close this tab');
  await expect(page.getByTestId('done-title')).toHaveText(/You received [\d,.]+ USDC on Base/, { timeout: 40_000 });

  // The exit quote: the full payout, the user's address, the taker's own Intents account.
  const exitQuote = mock.quotes.at(-1)!;
  expect(exitQuote.amount).toBe('937838000');
  expect(exitQuote.recipient).toBe(EVM_ADDRESS);
  await expect(page.getByText('What this swap revealed')).toBeVisible();
  await expect(page.getByText('Base transaction')).toBeVisible();

  await page.getByText('All steps and transactions').click();
  for (const label of ['Request to the maker (encrypted memo)', 'Maker locked the USDC in the escrow', 'Claim with your key half', 'USDC delivered']) {
    await expect(page.getByText(label)).toBeVisible();
  }
  await page.getByRole('button', { name: 'Start a new swap' }).click();
  await expect(page.getByRole('heading', { name: /Swap shielded ZEC/ })).toBeVisible();
});

test('the maker quotes below the minimum: the swap stops and the ZEC comes back', async ({ page }) => {
  await open(page, 'speed=100&fail=quote-low');
  await fillForm(page, { returnAddress: RETURN_ADDRESS });
  await createAndSave(page);
  await pay(page);
  await expect(page.getByTestId('refunded-title')).toHaveText('0.9998 ZEC is on its way back', { timeout: 30_000 });
  await expect(page.getByText('The maker’s firm price was below your minimum')).toBeVisible();
  await expect(page.getByText(/before the premium was paid/)).toBeVisible();
});

test('the maker never answers: refund to an address entered afterwards', async ({ page }) => {
  await open(page, 'speed=100&fail=no-quote');
  await fillForm(page);
  await createAndSave(page);
  await pay(page);
  await expect(now(page)).toHaveText('Where should your ZEC go?', { timeout: 30_000 });
  await expect(page.getByText('The maker did not answer')).toBeVisible();
  await page.getByLabel('Your Zcash address').fill('t1Rv4exT7bqhZqi2j7xz8bUHDMxwosrjADU');
  await page.getByRole('button', { name: 'Send my ZEC here' }).click();
  await expect(page.getByText(/transparent address/)).toBeVisible();
  await page.getByLabel('Your Zcash address').fill(RETURN_ADDRESS);
  await page.getByRole('button', { name: 'Send my ZEC here' }).click();
  await expect(page.getByTestId('refunded-title')).toBeVisible({ timeout: 30_000 });
});

test('the maker goes quiet after funding: the ZEC is recovered and refunded', async ({ page }) => {
  await open(page, 'speed=100&fail=maker-quiet');
  await fillForm(page, { returnAddress: RETURN_ADDRESS });
  await createAndSave(page);
  await pay(page);
  await expect(now(page)).toHaveText('Taking your ZEC back', { timeout: 40_000 });
  await expect(page.getByText('The maker stopped the swap')).toBeVisible();
  await expect(page.locator('.step-title', { hasText: 'Your ZEC comes back' })).toBeVisible();
  await expect(page.getByTestId('refunded-title')).toBeVisible({ timeout: 30_000 });
});

test('the maker disappears: the app claims through the punish path', async ({ page }) => {
  await open(page, 'speed=200&fail=maker-gone');
  await fillForm(page);
  await createAndSave(page);
  await pay(page);
  await expect(page.getByTestId('done-title')).toBeVisible({ timeout: 40_000 });
  await expect(page.getByText('After the deadline, because the maker went quiet')).toBeVisible();
});

test('the delivery fails once: USDC is safe, retry delivers', async ({ page }) => {
  await open(page, 'speed=100&fail=exit-fail');
  await fillForm(page);
  await createAndSave(page);
  await pay(page);
  await expect(now(page)).toHaveText('Delivery paused. Your USDC is safe.', { timeout: 30_000 });
  await expect(page.getByText(/came back to NEAR Intents under the key made in this browser/)).toBeVisible();
  await page.getByRole('button', { name: 'Try the delivery again' }).click();
  await expect(page.getByTestId('done-title')).toBeVisible({ timeout: 30_000 });
});

test('the delivery price falls below the minimum: nothing moves until the user accepts', async ({ page }) => {
  await open(page, 'speed=100&fail=exit-price');
  await fillForm(page);
  await createAndSave(page);
  await pay(page);
  await expect(now(page)).toHaveText('Delivery paused. Your USDC is safe.', { timeout: 30_000 });
  await expect(page.getByText(/The price for the delivery moved/)).toBeVisible();
  await page.getByRole('button', { name: /Accept at least/ }).click();
  // The page fetched a fresh price, which is back above the shown amount, so it delivers.
  await expect(page.getByTestId('done-title')).toBeVisible({ timeout: 30_000 });
});

test('closing the page mid-swap: the swap resumes after a reload', async ({ page }) => {
  await open(page, 'speed=50&pause=escrow_open');
  await fillForm(page);
  await createAndSave(page);
  await pay(page);
  await expect(now(page)).toHaveText('Sending your ZEC to the shared address', { timeout: 30_000 });
  await page.goto('/?speed=100');
  await expect(now(page)).toBeVisible();
  await expect(page.getByText('Your payment arrives')).toBeVisible();
  await expect(page.getByTestId('done-title')).toBeVisible({ timeout: 40_000 });
});

test('the claim window asks to keep the tab open and guards against leaving', async ({ page }) => {
  await open(page, 'speed=100&pause=committing');
  await fillForm(page);
  await createAndSave(page);
  await pay(page);
  await expect(now(page)).toHaveText('Claiming your USDC: step 1 of 2', { timeout: 30_000 });
  await expect(page.getByTestId('safety')).toContainText('Keep this tab open for about a minute');
  await expect(page).toHaveTitle(/\(5\/6\) Claiming your USDC/);
  const dialog = page.waitForEvent('dialog');
  void page.close({ runBeforeUnload: true });
  expect((await dialog).type()).toBe('beforeunload');
});

test('a cleared browser: restore from the recovery file and finish', async ({ page }) => {
  await open(page, 'speed=100&pause=funding_sent');
  await fillForm(page);
  const filePath = await createAndSave(page);
  await pay(page);
  await expect(page.getByText('ZEC sent to the shared address')).toBeVisible({ timeout: 30_000 });
  await demoControls(page, 'Simulate a cleared browser');
  await expect(page.getByRole('heading', { name: /Swap shielded ZEC/ })).toBeVisible();
  await page.getByRole('button', { name: /Restore a swap/ }).click();
  await page.getByTestId('recovery-input').setInputFiles({ name: 'bad.json', mimeType: 'application/json', buffer: Buffer.from('not json') });
  await expect(page.getByText('This file cannot be used')).toBeVisible();
  await page.getByTestId('recovery-input').setInputFiles(filePath);
  await expect(now(page)).toBeVisible();
  await page.goto('/?speed=100');
  await expect(page.getByTestId('done-title')).toBeVisible({ timeout: 40_000 });
});

test('restore in a fresh browser that never saw the swap', async ({ page, browser }) => {
  await open(page, 'speed=100');
  await fillForm(page);
  const filePath = await createAndSave(page);
  const fresh = await browser.newContext();
  const other = await fresh.newPage();
  await open(other);
  await other.getByRole('button', { name: /Restore a swap/ }).click();
  await other.getByTestId('recovery-input').setInputFiles(filePath);
  await expect(other.getByRole('heading', { name: /Send exactly 1.00 ZEC/ })).toBeVisible();
  await expect(other.getByTestId('deposit-address')).toHaveText(await page.getByTestId('deposit-address').innerText());
  await fresh.close();
});

test('form checks: amounts, addresses, NEAR Intents refusals and the keyboard token picker', async ({ page }) => {
  await open(page, '', { rejectRecipients: [SOL_ADDRESS] });
  const amount = page.getByLabel('You pay');
  await amount.fill('0.001');
  await expect(page.getByText('The maker takes at least 0.01 ZEC.')).toBeVisible();
  await amount.fill('3');
  await expect(page.getByText('The maker takes at most 2.00 ZEC per swap.')).toBeVisible();
  await amount.fill('1.123456789');
  await expect(page.getByText('ZEC has at most 8 decimal places.')).toBeVisible();
  await amount.fill('0.5');

  // Keyboard only: open the picker, search, choose SOL on Solana.
  await page.getByRole('button', { name: 'You receive' }).focus();
  await page.keyboard.press('Enter');
  await page.keyboard.type('sol solana');
  await expect(page.getByRole('option', { name: /SOL\s*Solana/ }).first()).toBeVisible();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('button', { name: 'You receive' })).toContainText('SOL');

  const address = page.getByLabel(/^Your SOL address on Solana/);
  await address.fill('0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed');
  await address.blur();
  await expect(page.getByText(/32 to 44 base58 characters/)).toBeVisible();
  await address.fill(SOL_ADDRESS);
  await expect(page.getByText(/NEAR Intents does not accept this address/)).toBeVisible();
  await page.getByRole('button', { name: 'Continue' }).click();
  await expect(page.getByRole('heading', { name: 'Save your recovery file' })).not.toBeVisible();

  await page.getByRole('button', { name: /Add a Zcash return address/ }).click();
  const ret = page.getByLabel(/Zcash return address/);
  await ret.fill('u1sc85xacmfvsy3l2ny320a4ppzmrssfamyevy67nmdv6dgd30jnjxn5mujeld6r4tuhkm0a4qqy6r58ede086kezc4q9n0xduhq0fym7x');
  await ret.blur();
  await expect(page.getByText(/This address is for Zcash mainnet/)).toBeVisible();
});

test('token list down: a clear error and a working retry', async ({ page }) => {
  let down = true;
  await mockOneClick(page);
  // Registered after the mock, so it is asked first.
  await page.route('https://1click.chaindefuser.com/v0/tokens', async (route) => {
    if (down) return route.fulfill({ status: 503, contentType: 'application/json', body: '{"message":"down"}' });
    return route.fallback();
  });
  await page.goto('/');
  await expect(page.getByText('The token list did not load')).toBeVisible();
  down = false;
  await page.getByRole('button', { name: 'Try again' }).click();
  await expect(page.getByRole('button', { name: 'You receive' })).toContainText('USDC');
});

test('the page stays responsive while the worker is busy proving', async ({ page }) => {
  await open(page, 'speed=20&cpu=1');
  await fillForm(page);
  await createAndSave(page);
  await page.evaluate(() => {
    const w = window as unknown as { maxGap: number };
    w.maxGap = 0;
    let last = performance.now();
    const loop = (t: number) => {
      w.maxGap = Math.max(w.maxGap, t - last);
      last = t;
      requestAnimationFrame(loop);
    };
    requestAnimationFrame(loop);
  });
  await pay(page);
  await expect(page.getByText('Price accepted, premium paid')).toBeVisible({ timeout: 40_000 });
  // Typing still works while the worker burns its CPU for the proofs.
  const maxGap = await page.evaluate(() => (window as unknown as { maxGap: number }).maxGap);
  expect(maxGap).toBeLessThan(250);
});

test('theme toggle switches and remembers the theme', async ({ page }) => {
  await open(page);
  await page.emulateMedia({ colorScheme: 'light' });
  await page.getByRole('button', { name: 'Use dark theme' }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await page.reload();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await expect(page.getByRole('button', { name: 'Use light theme' })).toBeVisible();
});
