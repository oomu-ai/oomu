import react from "@vitejs/plugin-react";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { configDefaults, defineConfig } from "vitest/config";

const dirname = path.dirname(fileURLToPath(import.meta.url));

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@": path.resolve(dirname, "src"),
    },
  },
  test: {
    css: false,
    environment: "jsdom",
    exclude: [...configDefaults.exclude, "sidecars/whatsapp-sidecar/test/**"],
    maxWorkers: 4,
    setupFiles: ["./src/test/setup.ts"],
  },
});
