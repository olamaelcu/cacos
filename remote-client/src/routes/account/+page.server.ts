import type { PageServerLoad } from './$types';
export const load: PageServerLoad = ({ url }) => ({
  did: url.searchParams.get('did') ?? '',
  handle: url.searchParams.get('handle'),
});