// The landing page at /, the app at /swap, and the routes between them.
import { expect, test } from '@playwright/test';
import { mockOneClick } from './oneclick-mock';
import { createAndSave, fillForm, nearWithoutMakerKey, open } from './helpers';

test('the landing page loads no engine; "Start a swap" opens the app at /swap, and Back returns', async ({ page }) => {
  await mockOneClick(page);
  const requests: string[] = [];
  page.on('request', (r) => requests.push(new URL(r.url()).pathname));
  await page.goto('/');
  await expect(page.getByRole('heading', { level: 1, name: 'Swap from fully shielded ZEC to 30+ chains' })).toBeVisible();
  for (const h of ['How a swap works', 'Who holds the money', 'What stays private and what is public', 'If something goes wrong', 'Status', 'Questions'])
    await expect(page.getByRole('heading', { level: 2, name: h })).toBeVisible();
  await page.waitForTimeout(500);
  expect(page.workers()).toHaveLength(0);
  expect(requests.filter((p) => p.includes('/engine/') || p.includes('worker') || p.includes('maker-directory') || p.startsWith('/config/'))).toEqual([]);

  await page.getByRole('main').getByRole('link', { name: 'Start a swap' }).first().click();
  await expect(page).toHaveURL(/\/swap$/);
  await expect(page.getByRole('heading', { name: /Swap shielded ZEC for a token/ })).toBeVisible();
  await page.goBack();
  await expect(page.getByRole('heading', { level: 1, name: 'Swap from fully shielded ZEC to 30+ chains' })).toBeVisible();
});

test('the demo link and old links with app settings open the app', async ({ page }) => {
  await mockOneClick(page);
  await page.goto('/');
  await page.getByRole('main').getByRole('link', { name: 'Try the simulated demo' }).first().click();
  await expect(page).toHaveURL(/\/swap\?demo=1$/);
  await expect(page.getByTestId('demo-banner')).toContainText('Demo. No real money moves.');
  await page.goto('/?demo=1');
  await expect(page.getByLabel('You pay')).toBeVisible();
  await page.goto('/?speed=20');
  await expect(page.getByLabel('You pay')).toBeVisible();
});

test('the landing page offers to open a swap this browser holds; the logo goes home', async ({ page }) => {
  await open(page, 'speed=1');
  await fillForm(page);
  await createAndSave(page);
  await page.getByRole('link', { name: 'Cloakroom Intents, home' }).click();
  await expect(page).toHaveURL(/\/$/);
  await expect(page.getByText('This browser holds a swap')).toBeVisible();
  await page.getByRole('main').getByRole('link', { name: 'Open your swap' }).first().click();
  await expect(page.getByRole('heading', { name: /Send exactly 1.00 ZEC/ })).toBeVisible();
});

test('the demo banner is one line; its details and the demo controls open on request', async ({ page }) => {
  await open(page, 'speed=20');
  const banner = page.getByTestId('demo-banner');
  const toggle = banner.getByRole('button', { name: 'Details' });
  await expect(toggle).toHaveAttribute('aria-expanded', 'false');
  await expect(banner.getByText(/Runs 20× faster/)).toBeHidden();
  await toggle.click();
  await expect(toggle).toHaveAttribute('aria-expanded', 'true');
  await expect(banner.getByText(/Runs 20× faster/)).toBeVisible();
  await banner.getByRole('button', { name: 'Demo controls' }).click();
  await expect(page.getByRole('dialog', { name: 'Demo controls' })).toBeVisible();
});

test('testnet: a one-line banner with details, and a failed maker check says so', async ({ page, context }) => {
  await nearWithoutMakerKey(context);
  await mockOneClick(page);
  await page.goto('/swap?config=testnet');
  const banner = page.getByTestId('network-banner');
  await expect(banner).toContainText('Testnet. Test money only.');
  await expect(banner.getByText(/test USDC token with no value/)).toBeHidden();
  await banner.getByRole('button', { name: 'Details' }).click();
  await expect(banner.getByText(/test USDC token with no value/)).toBeVisible();
  await expect(banner.getByText(/Real transactions, test money with no value/)).toBeVisible();

  await expect(page.getByRole('heading', { level: 1, name: 'The maker could not be verified' })).toBeVisible({ timeout: 60_000 });
  await expect(page.getByTestId('start-failure')).toContainText('does not swap with a maker it cannot verify');
  await expect(page.getByTestId('start-failure')).not.toContainText('may block Web Workers');
  await expect(page.getByTestId('failure-detail')).toContainText(/is not a key of maker\.zecret-3f109eea1e\.testnet/);
});

test('the chain strip shows a logo beside each chain name; the logos are hidden from screen readers', async ({ page }) => {
  await mockOneClick(page);
  await page.goto('/');
  const strip = page.getByRole('region', { name: /destination chains in the swap form/ });
  const items = strip.getByRole('listitem');
  await expect(items).toHaveCount(35);
  // The logos add no text: each item reads as the chain's name alone.
  await expect(items.first()).toHaveText('Ethereum');
  await expect(items.nth(34)).toHaveText('Aleo');
  const icons = strip.locator('.chains-list:not([aria-hidden]) .chain-icon');
  await expect(icons).toHaveCount(35);
  for (const i of await icons.all()) {
    await expect(i).toHaveAttribute('aria-hidden', 'true');
    const box = await i.boundingBox();
    expect(box && box.width >= 18 && box.width <= 28 && Math.abs(box.width - box.height) < 0.5).toBe(true);
  }
  await expect(strip.locator('.chains-list[aria-hidden="true"] li')).toHaveCount(35);
});
