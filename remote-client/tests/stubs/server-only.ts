// tests/stubs/server-only.ts
// Stub for SvelteKit's `server-only` marker package, which throws on
// accidental browser bundling. In tests we always run server-side, so
// a no-op is fine.
export {};
