import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// `bun dev` chooses a free port and hands it to both halves of the dev setup,
// this file through the environment and Tauri through its devUrl. A plain
// `bun run dev:web` starts on the default. `strictPort` stays on: if the port
// is taken between the check and this bind, failing loudly beats moving to a
// port the window is not pointed at, which loads a blank window.
const port = Number(process.env.LYRE_DEV_PORT ?? 1420);

// Tauri drives the dev server and expects a static frontend at build time.
// `clearScreen: false` keeps Rust compiler output visible.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  resolve: {
    alias: { "@": new URL("./src", import.meta.url).pathname },
  },
  server: {
    port,
    strictPort: true,
    host: "127.0.0.1",
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    target: "safari15",
    minify: "esbuild",
    cssMinify: "esbuild",
    sourcemap: false,
  },
});
