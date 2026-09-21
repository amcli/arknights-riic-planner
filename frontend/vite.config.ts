import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The dev server proxies /api to the Rust backend so the frontend can use
// relative URLs in both development and production (where a reverse proxy
// does the same job).
export default defineConfig({
  plugins: [react()],
  server: {
    // Honour PORT when a launcher assigns one; otherwise Vite's default.
    port: Number(process.env.PORT ?? 5173),
    proxy: {
      "/api": {
        target: process.env.AK_API_URL ?? "http://127.0.0.1:8080",
        changeOrigin: true,
      },
    },
  },
});
