// tests/stubs/app-navigation.ts
// Test stub for SvelteKit's $app/navigation. The screens test imports a
// component that uses goto(); we provide a no-op so component rendering
// doesn't require the full SvelteKit runtime.
export const goto = async () => {};
export const invalidate = async () => {};
export const invalidateAll = async () => {};
export const preloadData = async () => ({});
export const preloadCode = async () => {};
export const afterNavigate = () => {};
export const beforeNavigate = () => {};
export const disableScrollHandling = () => {};
export const pushState = () => {};
export const replaceState = () => {};