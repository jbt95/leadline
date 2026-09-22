import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Single-file-ish bundle served by the Rust binary: stable asset names,
// relative paths, no dev-only code paths.
export default defineConfig({
  plugins: [react()],
  base: "./",
  build: {
    outDir: "dist",
    emptyOutDir: true,
    assetsDir: "assets",
    sourcemap: false,
    chunkSizeWarningLimit: 1024,
    rollupOptions: {
      // Stable names: the Rust binary embeds these paths, so no hashes.
      output: {
        entryFileNames: "assets/app.js",
        chunkFileNames: "assets/app.js",
        assetFileNames: "assets/[name].[ext]",
      },
    },
  },
  server: {
    proxy: {
      "/api": "http://127.0.0.1:3000",
      "/health": "http://127.0.0.1:3000",
    },
  },
});
