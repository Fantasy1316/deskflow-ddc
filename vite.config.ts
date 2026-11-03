import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
export default defineConfig({
  plugins: [vue()],
  base: './',
  server: { port: 5174, strictPort: true, watch: { usePolling: true, interval: 300 } },
  test: { include: ['tests/**/*.test.ts'] },
} as Parameters<typeof defineConfig>[0])
