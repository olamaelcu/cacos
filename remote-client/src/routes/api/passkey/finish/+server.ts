import { json, type RequestHandler } from '@sveltejs/kit';
import { pds } from '$lib/pds-instance';
export const POST: RequestHandler = async ({ request, locals }) => {
  const b = await request.json();
  if (!b.credential_id) return new Response('missing credential_id', { status: 400 });
  const result = await pds().passkeyFinish({ ...b, device_id: locals.deviceId });
  return json(result);
};
