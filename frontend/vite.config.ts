import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

export default defineConfig({
  // Sub-path when hosted at https://<user>.github.io/<repo>/ (set by CI).
  base: process.env.VITE_BASE || '/',
  plugins: [vue()],
  server: {
    // `cargo run -- serve` in ../backend
    proxy: { '/api': 'http://127.0.0.1:8080' },
  },
})
