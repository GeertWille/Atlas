import { defineConfig, devices } from "@playwright/test";

const PORT = 1420;
/** The production build, for the specs that assert time budgets: React's development mode costs several times more. */
export const PRODUCTION_PORT = 1421;
const PRODUCTION_DIR = "node_modules/.cache/e2e-dist";
const PERFORMANCE_SPEC = /network-performance\.spec\.ts$/;

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  retries: 0,
  reporter: [["list"]],
  timeout: 30_000,
  expect: { timeout: 7_000 },
  // The time budgets of the network view are measured with nothing else running: their spec is a project that
  // starts once all the others have finished. `--no-deps` runs it on its own.
  projects: [
    { name: "main", testIgnore: PERFORMANCE_SPEC },
    { name: "performance", testMatch: PERFORMANCE_SPEC, dependencies: ["main"] },
  ],
  use: {
    baseURL: `http://localhost:${PORT}`,
    trace: "off",
    ...devices["Desktop Chrome"],
    viewport: { width: 1280, height: 820 },
    deviceScaleFactor: 1,
    launchOptions: { args: ["--no-sandbox"] },
    permissions: ["clipboard-read", "clipboard-write"],
  },
  webServer: [
    {
      command: "pnpm dev",
      url: `http://localhost:${PORT}`,
      reuseExistingServer: !process.env.CI,
      timeout: 60_000,
    },
    {
      command: `pnpm exec vite build --outDir ${PRODUCTION_DIR} --emptyOutDir --logLevel warn && pnpm exec vite preview --outDir ${PRODUCTION_DIR} --port ${PRODUCTION_PORT} --strictPort`,
      url: `http://localhost:${PRODUCTION_PORT}`,
      // Always built afresh: a server left running would serve the code of an earlier run.
      reuseExistingServer: false,
      timeout: 180_000,
    },
  ],
});
