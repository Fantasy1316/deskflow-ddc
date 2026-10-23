import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
export default defineConfig({
  plugins: [vue()],
  base: './',
  server: { port: 5173, strictPort: true },
  test: { include: ['tests/**/*.test.ts'] },
} as Parameters<typeof defineConfig>[0])
