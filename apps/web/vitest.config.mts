import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) },
  },
  test: {
    environment: "jsdom",
    setupFiles: ["./vitest.setup.ts"],
    include: ["src/**/*.test.{ts,tsx}"],
    css: false,
    // These tests drive real screens with userEvent. A long walk through the
    // setup wizard takes over five seconds on a loaded machine (CI, or a full
    // local run), so the five-second default failed tests that are not broken.
    // A hung test still fails here, just later.
    testTimeout: 20_000,
  },
});
