import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Unit tests for the renderer-level security boundary (issue #112): the
// MarkdownRenderer must hold against untrusted entry prose. jsdom, no globals.
export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
  },
});
