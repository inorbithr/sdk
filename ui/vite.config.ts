import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// `iohr ui serve` serves `dist/` with a strict CSP (no inline script or style), so the build must not inline any.
// In `iohr ui dev`, IOHR_UI_API names a running `iohr ui serve` (http://127.0.0.1:<port>) and /api goes to it.
const api = process.env.IOHR_UI_API;

export default defineConfig({
  plugins: [react()],
  build: { assetsInlineLimit: 0, sourcemap: false },
  server: {
    host: "127.0.0.1",
    strictPort: true,
    proxy: api ? { "/api": { target: api, changeOrigin: true } } : undefined,
  },
});
