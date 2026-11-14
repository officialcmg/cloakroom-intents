// The network switch in the header and the maker's liquidity on the swap form. The testnet
// tests run the real engine with NEAR RPC answered by this file: the maker's key is a
// full-access key, and the maker's balances are what each test sets.
import { expect, test, type BrowserContext, type Page } from '@playwright/test';
import { open, requireEngine } from './helpers';
import { mockOneClick } from './oneclick-mock';

/** micro-USDC the mocked NEAR RPC reports for the maker: [in the Verifier, plain]. */
async function nearRpc(context: BrowserContext, balances: { verifier: string; plain: string } | 'down') {
  requireEngine();
  const reads: string[] = [];
  const answer = (result: unknown) => ({ status: 200, contentType: 'application/json', headers: { 'access-control-allow-origin': '*' }, body: JSON.stringify({ jsonrpc: '2.0', id: 'zecret', result }) });
  const bytes = (v: unknown) => [...new TextEncoder().encode(JSON.stringify(v))];
  for (const host of ['rpc.testnet.fastnear.com', 'near-testnet.drpc.org', 'testnet-rpc.intea.rs']) {
    await context.route(`https://${host}/**`, async (r) => {
      if (r.request().method() === 'OPTIONS') return r.fulfill({ status: 204, headers: { 'access-control-allow-origin': '*', 'access-control-allow-headers': '*' } });
      const body = r.request().postDataJSON() as { method: string; params: Record<string, string> };
      const p = body.params ?? {};
      if (p.request_type === 'view_access_key') return r.fulfill(answer({ nonce: 1, permission: 'FullAccess', block_height: 1, block_hash: '11111111111111111111111111111111' }));
      if (body.method === 'block') return r.fulfill(answer({ header: { hash: 'H', height: 1, timestamp: 0 } }));
      if (p.request_type === 'call_function' && (p.method_name === 'mt_balance_of' || p.method_name === 'ft_balance_of')) {
        reads.push(`${host} ${p.method_name}`);
        if (balances === 'down') return r.fulfill({ status: 503, body: 'down', headers: { 'access-control-allow-origin': '*' } });
        return r.fulfill(answer({ result: bytes(p.method_name === 'mt_balance_of' ? balances.verifier : balances.plain), logs: [], block_height: 1, block_hash: 'H' }));
      }
      return r.fulfill({ status: 503, body: 'not mocked', headers: { 'access-control-allow-origin': '*' } });
    });
  }
  return reads;
}

async function openTestnetForm(page: Page) {
  await mockOneClick(page);
  await page.goto('/swap?config=testnet');
  await expect(page.getByLabel('You pay')).toBeVisible({ timeout: 60_000 });
}

test('testnet: the maker liquidity is read from NEAR; a swap above it is blocked before payment', async ({ page, context }) => {
  const reads = await nearRpc(context, { verifier: '1000000', plain: '200000' });
  await openTestnetForm(page);
  await expect(page.getByTestId('liquidity-value')).toHaveText('Maker liquidity: 1.20 test USDC');
  expect(reads).toEqual(expect.arrayContaining(['rpc.testnet.fastnear.com mt_balance_of', 'rpc.testnet.fastnear.com ft_balance_of']));

  await page.getByRole('button', { name: 'What is maker liquidity?' }).click();
  await expect(page.getByText(/its balance in NEAR Intents plus its own token balance, read from NEAR/)).toBeVisible();

  // 0.05 ZEC sells about 0.0496 ZEC at 30 test USDC: about 1.48, more than 1.20.
  await page.getByLabel('You pay').fill('0.05');
  await expect(page.getByTestId('liquidity-short')).toContainText('The maker holds 1.20 test USDC now. This swap needs about 1.48.');
  await expect(page.getByRole('button', { name: 'Continue' })).toBeDisabled();

  // A smaller swap fits.
  await page.getByLabel('You pay').fill('0.02');
  await expect(page.getByTestId('liquidity-value')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Continue' })).toBeEnabled();
});

test('testnet: when NEAR RPC fails, the liquidity line stays empty and blocks nothing', async ({ page, context }) => {
  await nearRpc(context, 'down');
  await openTestnetForm(page);
  const line = page.getByTestId('liquidity');
  await expect(line).toBeVisible();
  await page.getByLabel('You pay').fill('0.05');
  await expect(page.getByTestId('price-line')).toContainText('You receive at least');
  await expect(page.getByTestId('liquidity-value')).toHaveCount(0);
  await expect(page.getByTestId('liquidity-short')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Continue' })).toBeEnabled();
});

test('demo: a fixed, labelled liquidity from the simulated directory, and no NEAR request', async ({ page }) => {
  const near: string[] = [];
  await page.route(/near|fastnear|intea/, (r) => {
    near.push(r.request().url());
    return r.abort();
  });
  await open(page);
  await expect(page.getByTestId('liquidity-value')).toHaveText('Maker liquidity: 5,000.00 USDC (simulated)');
  expect(near.filter((u) => !u.includes('1click'))).toEqual([]);
});

test('network switch: only in the app; testnet is the current page, mainnet is off with its reason', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('navigation', { name: 'Network' })).toHaveCount(0);

  await open(page, 'demo=1');
  const sw = page.getByRole('navigation', { name: 'Network' });
  const current = sw.getByRole('link', { name: 'Testnet' });
  await expect(current).toHaveAttribute('aria-current', 'page');
  await expect(current).toHaveAttribute('href', '/swap');
  await expect(sw.getByRole('link', { name: /Mainnet/ })).toHaveCount(0);
  await expect(page.getByTestId('net-mainnet-off')).toHaveAttribute('title', 'Mainnet is not deployed yet');
  await expect(page.getByTestId('net-mainnet-off')).toContainText('Mainnet is not deployed yet');
});
