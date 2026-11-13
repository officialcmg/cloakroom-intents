// Security review 3, NV3-02: one tab runs the engine of this browser at a time. Two pages
// of one browser context share IndexedDB and Web Locks, as two tabs do. Against the
// simulated engine; the real engine takes the same lock (src/app/useZecret.ts) and also
// refuses an older tab's writes (src/engine/real/idb.ts, unit-tested).
import { expect, test } from '@playwright/test';
import { mockOneClick } from './oneclick-mock';
import { createAndSave, fillForm, open, pay } from './helpers';

test('a second tab shows that the swap is open in another tab, runs no engine, and takes over when the first closes', async ({ context }) => {
  const first = await context.newPage();
  await open(first, 'speed=100&pause=committing');
  await fillForm(first);
  await createAndSave(first);
  expect(first.workers()).toHaveLength(1);

  const second = await context.newPage();
  await mockOneClick(second);
  await second.goto('/?speed=100&pause=committing');
  await expect(second.getByRole('heading', { name: 'This swap is open in another tab' })).toBeVisible();
  await expect(second.getByTestId('other-tab')).toContainText('this tab then continues the swap by itself');
  // No engine runs in the second tab: no worker, no swap screen.
  expect(second.workers()).toHaveLength(0);
  await expect(second.getByTestId('deposit-address')).toHaveCount(0);

  // The first tab goes on as before.
  await pay(first);
  await expect(first.getByTestId('now-title')).toBeVisible();
  await second.waitForTimeout(1_000);
  expect(second.workers()).toHaveLength(0);

  // Closing the first tab frees the lock: the second tab starts its engine and shows the swap.
  await first.close();
  await expect(second.getByTestId('now-title')).toBeVisible({ timeout: 20_000 });
  await expect(second.getByTestId('other-tab')).toHaveCount(0);
  expect(second.workers()).toHaveLength(1);
});

test('a production build refuses network settings from a link', async ({ page }) => {
  await open(page, 'config=https://attacker.example/c.json');
  await expect(page.getByRole('heading', { name: 'The network settings did not load' })).toBeVisible();
  await expect(page.getByText(/loads only its own network settings/)).toBeVisible();
  await expect(page.getByTestId('config-warning')).toHaveCount(0);
});
