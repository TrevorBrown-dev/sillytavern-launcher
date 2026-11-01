import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Set by `tauri dev` when developing against a remote device; unused on desktop.
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [svelte()],
  // Tauri prints its own output; keep Vite from wiping the terminal.
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    // WebView2 on Windows is evergreen Chromium; no need to transpile further down.
    target: "chrome105",
    sourcemap: false,
  },
});
