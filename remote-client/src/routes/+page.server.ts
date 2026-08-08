// src/routes/+page.server.ts
import { fail, redirect, type Actions } from '@sveltejs/kit';
import { pds } from '$lib/pds-instance';
import { InvalidStateError, PdsError } from '$lib/server/pds/types';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ url, locals }) => {
  const rqid = url.searchParams.get('rqid');
  const state = url.searchParams.get('state');
  if (!rqid || !state) throw redirect(302, '/');
  try {
    const payload = await pds().request({ rqid, state, device_id: locals.deviceId });
    return { rqid, state: payload.state ?? state, payload, deviceId: locals.deviceId };
  } catch (e) {
    if (e instanceof InvalidStateError) throw redirect(302, '/');
    throw e;
  }
};

async function callAndRerender(fn: () => Promise<unknown>) {
  try {
    const result = await fn();
    return { ok: true as const, result };
  } catch (e) {
    if (e instanceof InvalidStateError) throw redirect(302, '/');
    if (e instanceof PdsError) return fail(400, { error: e.error, error_description: e.error_description });
    throw e;
  }
}

export const actions: Actions = {
  signIn: async ({ request, locals }) => {
    const f = await request.formData();
    const body = {
      rqid: String(f.get('rqid')), state: String(f.get('state')), device_id: String(f.get('device_id')),
      identifier: String(f.get('identifier')), password: String(f.get('password')),
    };
    return callAndRerender(() => pds().signIn(body));
  },
  select: async ({ request, locals }) => {
    const f = await request.formData();
    const body = {
      rqid: String(f.get('rqid')), state: String(f.get('state')), device_id: String(f.get('device_id')),
      did: String(f.get('did')),
    };
    return callAndRerender(() => pds().select(body));
  },
  createAccount: async ({ request, locals }) => {
    const f = await request.formData();
    const invite = f.get('invite_code');
    const body = {
      rqid: String(f.get('rqid')), state: String(f.get('state')), device_id: String(f.get('device_id')),
      handle: String(f.get('handle')), email: String(f.get('email')), password: String(f.get('password')),
      invite_code: typeof invite === 'string' && invite.length ? invite : undefined,
    };
    return callAndRerender(() => pds().createAccount(body));
  },
  accept: async ({ request }) => {
    const f = await request.formData();
    const body = {
      rqid: String(f.get('rqid')), state: String(f.get('state')), device_id: String(f.get('device_id')),
      did: String(f.get('did')),
    };
    const { redirect_url } = await pds().accept(body);
    throw redirect(302, redirect_url);
  },
  reject: async ({ request }) => {
    const f = await request.formData();
    const body = {
      rqid: String(f.get('rqid')), state: String(f.get('state')), device_id: String(f.get('device_id')),
    };
    const { redirect_url } = await pds().reject(body);
    throw redirect(302, redirect_url);
  },
};
