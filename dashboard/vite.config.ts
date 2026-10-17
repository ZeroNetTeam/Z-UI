import { fileURLToPath, URL } from "node:url";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

// Relative asset URLs: the panel serves the page under any secret path and
// tells it where with a <base> tag, so one build works everywhere.
export default defineConfig({
  base: "./",
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) },
  },
  server: {
    // `cargo run -p z-panel` serves the API on 2053.
    proxy: { "/api": "http://127.0.0.1:2053" },
  },
  build: { outDir: "dist", emptyOutDir: true, sourcemap: false },
  test: { environment: "jsdom" },
});
