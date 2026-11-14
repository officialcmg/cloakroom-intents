import { defineConfig, devices } from '@playwright/test';

// The mainnet build switch, switched on (docs/MAINNET.md): a separate build in dist-mainnet
// on its own port, with VITE_ZECRET_ALLOW_MAINNET=1, VITE_ZECRET_CONFIG=mainnet and a
// stand-in escrow. Run: npx playwright test -c playwright.mainnet.config.ts
// The default config (playwright.config.ts) tests the default build, which refuses mainnet.
const PORT = 4174;
const env = 'VITE_ZECRET_ALLOW_MAINNET=1 VITE_ZECRET_CONFIG=mainnet VITE_MAINNET_ESCROW=escrow.zecret.near VITE_MAINNET_ESCROW_CODE_HASH=DuBGKiy8dYPFkVHXLj4VFHLZiZ4ngQzssRUbXTCuPKq1 VITE_ZECRET_TESTNET_URL=https://testnet.example.org';

export default defineConfig({
  testDir: './e2e/mainnet',
  timeout: 60_000,
  expect: { timeout: 15_000 },
  workers: 2,
  reporter: [['list']],
  use: { baseURL: `http://localhost:${PORT}`, trace: 'off' },
  webServer: {
    command: `${env} npx vite build --logLevel error --outDir dist-mainnet && npx vite preview --outDir dist-mainnet --port ${PORT} --strictPort`,
    url: `http://localhost:${PORT}`,
    reuseExistingServer: false,
    timeout: 180_000,
  },
  projects: [
    { name: 'desktop', use: { ...devices['Desktop Chrome'], viewport: { width: 1280, height: 900 } } },
    { name: 'phone', use: { ...devices['Pixel 7'] } },
  ],
});
