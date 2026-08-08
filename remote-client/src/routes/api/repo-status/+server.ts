import { json, type RequestHandler } from '@sveltejs/kit';
import { pds } from '$lib/pds-instance';

export const GET: RequestHandler = async ({ url }) => {
  const did = url.searchParams.get('did');
  if (!did) return new Response('missing did', { status: 400 });
  try {
    return json(await pds().repoStatus(did));
  } catch (e) {
    return new Response('unavailable', { status: 503 });
  }
};