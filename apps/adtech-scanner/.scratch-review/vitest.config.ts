import { defineConfig } from "vitest/config";
export default defineConfig({
  test: { root: import.meta.dirname, include: ["*.spec.ts"], environment: "node", fileParallelism: false, testTimeout: 400_000, hookTimeout: 90_000 },
});
