// The default build (the live testnet site) does not run on mainnet: a link cannot switch
// it (docs/MAINNET.md, the build switch). The switched build is tested in e2e/mainnet/.
import { expect, test } from '@playwright/test';
import { open } from './helpers';

test('the default build refuses ?config=mainnet and moves nothing', async ({ page }) => {
  const external: string[] = [];
  page.on('request', (r) => {
    const u = new URL(r.url());
    if (u.hostname !== 'localhost') external.push(u.origin);
  });
  await open(page, 'config=mainnet');
  await expect(page.getByRole('heading', { name: 'This site does not run on mainnet' })).toBeVisible();
  await expect(page.getByTestId('failure-detail')).toContainText('This build of the page does not run on mainnet.');
  await expect(page.getByRole('button', { name: 'Try again' })).toHaveCount(0);
  await expect(page.getByTestId('mainnet-notice')).toHaveCount(0);
  expect(external.filter((o) => /mainnet|chainsafe|fastnear/.test(o))).toEqual([]);
});

test('the default landing page says testnet and links the faucet', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByText('Testnet · test money only')).toBeVisible();
  await expect(page.getByRole('link', { name: 'Fauzec faucet' }).first()).toBeVisible();
  await expect(page.getByText(/Mainnet · real money/)).toHaveCount(0);
});
