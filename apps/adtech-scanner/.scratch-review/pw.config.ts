import { defineConfig, devices } from "@playwright/test";
export default defineConfig({ testDir: ".", testMatch: /.*\.pw\.ts/, reporter: [["list"]], use: { ...devices["Desktop Chrome"], viewport: { width: 1280, height: 820 }, launchOptions: { args: ["--no-sandbox"] } } });
