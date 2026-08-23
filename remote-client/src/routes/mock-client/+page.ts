// src/routes/mock-client/+page.ts
// Local stand-in for a third-party OAuth client. Receives the redirect from
// `/?mock=consent` (Allow → success, Deny → error) and renders the captured
// params. Dev-only — production builds 404 this route.
import { error } from '@sveltejs/kit';
import type { PageLoad } from './$types';

export const load: PageLoad = ({ url }) => {
  if (!import.meta.env.DEV) throw error(404, 'Not found');
  return {
    code: url.searchParams.get('code'),
    state: url.searchParams.get('state'),
    error: url.searchParams.get('error'),
    error_description: url.searchParams.get('error_description'),
  };
};
