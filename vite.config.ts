import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// Tauri drives the dev server, so the port is fixed and must not drift.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: false,
    watch: {
      // Rust sources are rebuilt by cargo, not by Vite.
      ignored: ['**/src-tauri/**'],
    },
  },
  build: {
    // WebView2 on Windows 10+ is evergreen Chromium.
    target: 'chrome110',
    sourcemap: false,
    chunkSizeWarningLimit: 900,
  },
})
