import { json, type RequestHandler } from '@sveltejs/kit';
import { pds } from '$lib/pds-instance';
export const POST: RequestHandler = async ({ request, locals }) => {
  const b = await request.json();
  if (!b.rqid || !b.state) return new Response('missing rqid/state', { status: 400 });
  const result = await pds().passkeyStart({ ...b, device_id: locals.deviceId });
  return json(result);
};
