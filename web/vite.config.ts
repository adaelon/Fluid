import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

// Dev: Vite (5173) proxies /api to the Rust backend (127.0.0.1:7878), so the
// frontend stays cross-origin-free without touching the backend (S3, ADR-0014).
export default defineConfig({
  plugins: [vue()],
  server: {
    port: 5173,
    proxy: {
      // `ws: true` so the /api/generate WebSocket (S7) is proxied too, not just HTTP.
      '/api': { target: 'http://127.0.0.1:7878', changeOrigin: true, ws: true },
    },
  },
})
