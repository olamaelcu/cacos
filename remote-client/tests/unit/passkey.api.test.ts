import { describe, it, expect, vi } from 'vitest';
import { POST as start } from '../../src/routes/api/passkey/start/+server';
import { POST as finish } from '../../src/routes/api/passkey/finish/+server';

function makeEvent(body: unknown, locals: any = { deviceId: 'd' }) {
  return { request: new Request('http://x', { method: 'POST', body: JSON.stringify(body), headers: { 'content-type': 'application/json' } }), locals, url: new URL('http://x'), params: {}, route: { id: '' }, cookies: {} } as any;
}

describe('passkey api routes', () => {
  it('start returns 400 when rqid missing', async () => {
    const res = await start(makeEvent({}));
    expect(res.status).toBe(400);
  });

  it('start proxies to PDS and returns its JSON', async () => {
    const fakeFetch = vi.fn().mockResolvedValue(new Response(JSON.stringify({ challenge: 'c', rp_id: 'localhost', user_verification: 'required' }), { headers: { 'content-type': 'application/json' } }));
    const realFetch = globalThis.fetch;
    globalThis.fetch = fakeFetch as any;
    try {
      const res = await start(makeEvent({ rqid: 'r', state: 's', mode: 'entryway' }));
      expect(res.status).toBe(200);
      expect(await res.json()).toMatchObject({ challenge: 'c' });
      expect(fakeFetch).toHaveBeenCalledWith(expect.stringContaining('/oauth/remote/passkey/start'), expect.objectContaining({ method: 'POST' }));
    } finally { globalThis.fetch = realFetch; }
  });

  it('finish returns 400 when credential_id missing', async () => {
    const res = await finish(makeEvent({}));
    expect(res.status).toBe(400);
  });
});
