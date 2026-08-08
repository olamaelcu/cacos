import { json, type RequestHandler } from '@sveltejs/kit';
import { pds } from '$lib/pds-instance';
export const POST: RequestHandler = async ({ request, locals }) => {
  const b = await request.json();
  if (!b.did) return new Response('missing did', { status: 400 });
  const result = await pds().passkeyRegisterStart({ ...b, device_id: locals.deviceId });
  return json(result);
};
