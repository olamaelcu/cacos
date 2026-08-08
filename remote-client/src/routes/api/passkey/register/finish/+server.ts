import { json, type RequestHandler } from '@sveltejs/kit';
import { pds } from '$lib/pds-instance';
export const POST: RequestHandler = async ({ request, locals }) => {
  const b = await request.json();
  if (!b.attestation_object) return new Response('missing attestation_object', { status: 400 });
  const result = await pds().passkeyRegisterFinish({ ...b, device_id: locals.deviceId });
  return json(result);
};
