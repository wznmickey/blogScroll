import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

export default defineConfig({
  // Sub-path when hosted at https://<user>.github.io/<repo>/ (set by CI).
  base: process.env.VITE_BASE || '/',
  plugins: [vue()],
  // 设置 → 诊断信息 shows which build is running: the commit (in CI) and when.
  define: {
    __BUILD__: JSON.stringify(
      `${process.env.GITHUB_SHA?.slice(0, 7) ?? 'local'} · ${new Date().toISOString().slice(0, 16).replace('T', ' ')} UTC`,
    ),
  },
  server: {
    // `cargo run -- serve` in ../backend
    proxy: { '/api': 'http://127.0.0.1:8080' },
  },
})
