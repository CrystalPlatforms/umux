/// <reference types="vitest/config" />
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import { fileURLToPath } from 'node:url'

export default defineConfig({
  plugins: [react()],
  // Splashscreen (quickupdate 2026-09-13): a second, dependency-free page the
  // splash window loads while the React bundle boots. Both entries land in
  // dist/ (frontendDist in tauri.conf.json).
  build: {
    rollupOptions: {
      input: {
        main: fileURLToPath(new URL('./index.html', import.meta.url)),
        splashscreen: fileURLToPath(new URL('./splashscreen.html', import.meta.url)),
      },
    },
  },
  server: {
    watch: {
      ignored: ['**/src-tauri/target/**']
    }
  },
  test: {
    environment: 'jsdom',
    setupFiles: './src/setupTests.ts',
    globals: true,
  },
})
