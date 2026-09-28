import { defineConfig } from 'vitest/config'

// Only pure modules are tested here: helpers, comparators, decisions. Anything
// that touches Tauri IPC is covered end to end by the Rust smoke test instead.
export default defineConfig({
  test: {
    environment: 'node',
    include: ['src/**/*.test.ts'],
  },
})
