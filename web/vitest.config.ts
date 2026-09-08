import { defineConfig, defaultExclude } from "vitest/config";
import react from "@vitejs/plugin-react";

// Unit tests for the renderer-level security boundary (issue #112): the
// MarkdownRenderer must hold against untrusted entry prose. jsdom, no globals.
export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
    // macOS writes AppleDouble sidecars (`._MarkdownRenderer.test.tsx`) for
    // xattr'd files on volumes without native xattr support. They match the
    // include glob and die in transform — git-ignored junk is not a test
    // (issue #131; the suite is red on such volumes without this).
    exclude: [...defaultExclude, "**/._*"],
  },
});
