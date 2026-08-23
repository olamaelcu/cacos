import { mockSession } from '$lib/server/mock-data';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = ({ url }) => {
  if (import.meta.env.DEV && url.searchParams.has('mock')) {
    const session = mockSession();
    return { did: session.did, handle: session.handle, email: session.email };
  }
  return {
    did: url.searchParams.get('did') ?? '',
    handle: url.searchParams.get('handle'),
    email: url.searchParams.get('email'),
  };
};
