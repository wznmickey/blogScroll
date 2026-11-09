import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

export default defineConfig({
  plugins: [vue()],
  server: {
    // `cargo run -- serve` in ../backend
    proxy: { '/api': 'http://127.0.0.1:8080' },
  },
})
