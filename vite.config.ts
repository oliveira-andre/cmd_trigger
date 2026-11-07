import { defineConfig } from "vite";
// @ts-expect-error type error without @types/node package
import process from "node:process";
const host = process.env.TAURI_DEV_HOST;

// One page; each window picks its view from the URL hash.
export default defineConfig(() => ({
  clearScreen: false,
  server: {
    port: 1450,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1451 } : undefined,
    watch: { ignored: ["**/src-tauri/**", "**/crates/**", "**/target/**", "**/.dev-data/**"] },
  },
}));
