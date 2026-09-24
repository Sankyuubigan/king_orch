import { defineConfig } from "vite";

export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    // Проект windows-only (nsis/vcvarsall): дефолт chrome105. safari13 ронял
    // ручной `npm run build` (без TAURI_ENV_PLATFORM) на BigInt-литералах
    // transformers — фронт не собирался, приложение гоняло старый бандл.
    target: (process.env.TAURI_ENV_PLATFORM ?? "windows") === "windows" ? "chrome105" : "safari13",
    minify: !process.env.TAURI_ENV_DEBUG ? "esbuild" : false,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
    chunkSizeWarningLimit: 2000,
  },
});