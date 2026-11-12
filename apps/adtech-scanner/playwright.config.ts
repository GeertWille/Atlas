import { defineConfig, devices } from "@playwright/test";

const PORT = 1420;

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  retries: 0,
  reporter: [["list"]],
  timeout: 30_000,
  expect: { timeout: 7_000 },
  use: {
    baseURL: `http://localhost:${PORT}`,
    trace: "off",
    ...devices["Desktop Chrome"],
    viewport: { width: 1280, height: 820 },
    deviceScaleFactor: 1,
    launchOptions: { args: ["--no-sandbox"] },
    permissions: ["clipboard-read", "clipboard-write"],
  },
  webServer: {
    command: "pnpm dev",
    url: `http://localhost:${PORT}`,
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
});
