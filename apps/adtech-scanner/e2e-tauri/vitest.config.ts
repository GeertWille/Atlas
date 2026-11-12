import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    root: import.meta.dirname,
    include: ["*.spec.ts"],
    environment: "node",
    // Every file drives its own app window on the one display.
    fileParallelism: false,
    testTimeout: 180_000,
    hookTimeout: 60_000,
    bail: 1,
  },
});
