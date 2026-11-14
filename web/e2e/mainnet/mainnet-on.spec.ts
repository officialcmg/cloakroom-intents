// A build made with VITE_ZECRET_ALLOW_MAINNET=1 and VITE_ZECRET_CONFIG=mainnet
// (playwright.mainnet.config.ts). Every request outside this page is blocked: these tests
// check the page, not the chains. The maker directory is a fixture; its signature is not
// real, so the engine stops at the maker check, after the page has shown its notices.
import { readFileSync } from 'node:fs';
import { expect, test, type Page } from '@playwright/test';

const DIRECTORY = readFileSync(new URL('../fixtures/mainnet-directory.json', import.meta.url), 'utf8');

async function offline(page: Page) {
  const external: string[] = [];
  await page.route('**/*', (route) => {
    const u = new URL(route.request().url());
    if (u.hostname === 'localhost') {
      if (u.pathname === '/config/mainnet-directory.json') return route.fulfill({ status: 200, contentType: 'application/json', body: DIRECTORY });
      return route.continue();
    }
    external.push(u.origin);
    return route.abort();
  });
  return external;
}

test('the swap app loads the mainnet settings and shows the real-money notice until it is confirmed', async ({ page }) => {
  await offline(page);
  await page.goto('/swap');
  const notice = page.getByTestId('mainnet-notice');
  await expect(notice).toContainText('Real money. Unaudited hackathon software.');
  await expect(notice).toContainText('Zcash mainnet and NEAR mainnet. Real ZEC and real USDC move.');
  await expect(notice).toContainText('This site takes at most 0.005 ZEC per swap.');
  await expect(notice).toContainText('No security audit was done');
  await expect(notice).toContainText('had not run with real funds');
  await expect(notice).toContainText('one NEAR RPC endpoint');
  await expect(page.getByTestId('network-banner')).toHaveCount(0);
  await expect(page.getByTestId('demo-banner')).toHaveCount(0);
  const sw = page.getByRole('navigation', { name: 'Network' });
  await expect(sw.getByRole('link', { name: 'Mainnet' })).toHaveAttribute('aria-current', 'page');
  // The build sets VITE_ZECRET_TESTNET_URL (playwright.mainnet.config.ts): a plain link to that site's app, no query.
  await expect(sw.getByRole('link', { name: /^Testnet/ })).toHaveAttribute('href', 'https://testnet.example.org/swap');

  await notice.getByRole('button', { name: 'I understand' }).click();
  await expect(notice).toHaveCount(0);
  const banner = page.getByTestId('mainnet-banner');
  await expect(banner).toContainText('Mainnet. Real money. Unaudited hackathon software, at most 0.005 ZEC per swap.');
  await page.reload();
  await expect(page.getByTestId('mainnet-banner')).toBeVisible();
  await expect(page.getByTestId('mainnet-notice')).toHaveCount(0);
});

test('the mainnet build takes ?config=mainnet and still refuses outside settings', async ({ page }) => {
  await offline(page);
  await page.goto('/swap?config=mainnet');
  await expect(page.getByTestId('mainnet-notice')).toBeVisible();
  await page.goto('/swap?config=https://attacker.example/c.json');
  await expect(page.getByTestId('failure-detail')).toContainText(/loads only its own network settings \(local, testnet, mainnet\)/);
});

test('the mainnet landing page says real money and has no faucet hint', async ({ page }) => {
  await offline(page);
  await page.goto('/');
  await expect(page.getByText('Mainnet · real money · unaudited')).toBeVisible();
  await expect(page.getByText(/runs on Zcash mainnet and NEAR mainnet with real money/)).toBeVisible();
  await expect(page.getByRole('link', { name: 'Fauzec faucet' })).toHaveCount(0);
  await expect(page.getByText('Testnet · test money only')).toHaveCount(0);
  await expect(page.getByRole('heading', { name: 'Try it on mainnet' })).toBeVisible();
  await expect(page.getByRole('navigation', { name: 'Network' })).toHaveCount(0);
});
