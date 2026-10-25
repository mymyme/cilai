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
  build: {
    rolldownOptions: {
      output: {
        codeSplitting: {
          maxSize: 450_000,
          groups: [
            { name: "react", test: /[\\/]node_modules[\\/](?:react|react-dom|scheduler)[\\/]/, priority: 20, includeDependenciesRecursively: false },
            { name: "markdown", test: /[\\/]node_modules[\\/](?:react-markdown|remark-[^\\/]+|rehype-[^\\/]+|micromark[^\\/]*|mdast-util-[^\\/]+|hast-util-[^\\/]+|unified|vfile[^\\/]*|unist-util-[^\\/]+)[\\/]/, priority: 10, includeDependenciesRecursively: false },
          ],
        },
      },
    },
  },
  worker: { format: "es" },
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
