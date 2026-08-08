import { defineConfig } from 'vite';
import { sveltekit } from '@sveltejs/kit/vite';
import mkcert from 'vite-plugin-mkcert';

export default defineConfig({
  plugins: [mkcert({
    force: true,
    autoUpgrade: true,
  }), sveltekit()],
  server: {
    port: 5194,
    strictPort: true,
  },
});
