import { svelte } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  clearScreen: false,
  plugins: [svelte()],
  resolve: {
    conditions: ['browser']
  },
  server: {
    fs: {
      allow: ['.', '../../fixtures/markdown']
    },
    port: 1420,
    strictPort: true
  },
  test: {
    environment: 'jsdom',
    include: ['src/**/*.test.ts']
  }
});
