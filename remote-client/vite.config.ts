import { defineConfig } from 'vite';
import { sveltekit } from '@sveltejs/kit/vite';
import mkcert from 'vite-plugin-mkcert';

export default defineConfig({
  plugins: [mkcert(), sveltekit()],
  server: {
    port: 5194,
    strictPort: true,
  },
});