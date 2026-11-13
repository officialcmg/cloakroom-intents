import { defineConfig, devices } from '@playwright/test';

// Chromium is preinstalled at /opt/pw-browsers (PLAYWRIGHT_BROWSERS_PATH). The 1Click API
// is mocked in every test (e2e/oneclick-mock.ts), so the tests need no network.
//
// The default run uses Chromium only. ZECRET_BROWSERS=all adds the same desktop tests in
// Firefox and WebKit (projects `firefox` and `webkit`). Those browsers must be installed
// first: npx playwright install firefox webkit.
const PORT = 4173;
const ALL_BROWSERS = process.env.ZECRET_BROWSERS === 'all';

export default defineConfig({
  testDir: './e2e',
  timeout: 60_000,
  expect: { timeout: 15_000 },
  fullyParallel: true,
  workers: 4,
  reporter: [['list']],
  use: {
    baseURL: `http://localhost:${PORT}`,
    trace: 'off',
    acceptDownloads: true,
  },
  webServer: {
    command: `npx vite build --logLevel error && npx vite preview --port ${PORT} --strictPort`,
    url: `http://localhost:${PORT}`,
    reuseExistingServer: true,
    timeout: 120_000,
  },
  projects: [
    {
      name: 'desktop',
      testIgnore: /screenshots\.spec/,
      use: { ...devices['Desktop Chrome'], viewport: { width: 1280, height: 900 } },
    },
    {
      name: 'phone',
      // responsive.spec sets its own narrow viewports, so it runs once, in the desktop project.
      testIgnore: /screenshots\.spec|responsive\.spec/,
      use: { ...devices['Pixel 7'] },
    },
    {
      name: 'screenshots',
      testMatch: /screenshots\.spec/,
      use: { ...devices['Desktop Chrome'] },
    },
    ...(ALL_BROWSERS
      ? [
          {
            name: 'firefox',
            testIgnore: /screenshots\.spec/,
            use: { ...devices['Desktop Firefox'], viewport: { width: 1280, height: 900 } },
          },
          {
            name: 'webkit',
            testIgnore: /screenshots\.spec/,
            use: { ...devices['Desktop Safari'], viewport: { width: 1280, height: 900 } },
          },
        ]
      : []),
  ],
});
