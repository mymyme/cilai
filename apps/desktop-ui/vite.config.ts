import { defineConfig } from "vitest/config";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import { pdfAssets } from "./pdf-assets";

const pdfJsRoot = resolve(
  dirname(fileURLToPath(import.meta.url)),
  "../../node_modules/pdfjs-dist",
);

export default defineConfig({
  plugins: [
    react(),
    pdfAssets(pdfJsRoot),
  ],
  clearScreen: false,
  server: {
    strictPort: true,
    host: "127.0.0.1",
    port: 1421,
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.ts", "src/**/*.test.tsx"],
    setupFiles: ["src/test-setup.ts"],
    restoreMocks: true,
  },
});
