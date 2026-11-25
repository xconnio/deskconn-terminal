import { fileURLToPath, URL } from 'node:url'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

// The terminal, stores and services are deskconn-web-app's own source
// (installed from GitHub), imported as is. vue and pinia must be one copy for both.
export default defineConfig({
  plugins: [vue()],
  resolve: {
    alias: { '@': fileURLToPath(new URL('./node_modules/deskconn-web-app/src', import.meta.url)) },
    dedupe: ['vue', 'pinia'],
  },
  optimizeDeps: { exclude: ['@xconnio/xconn'] },
  server: { port: 5174, strictPort: true },
})
