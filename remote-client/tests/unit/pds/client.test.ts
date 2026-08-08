import { describe, it, expect, vi } from 'vitest';
import { createPdsClient, InvalidStateError, PdsError } from '../../../src/lib/server/pds/client';

describe('PdsClient', () => {
  const base = 'https://pds.example';
  const token = 'secret';

  function jsonResponse(body: unknown, status = 200): Response {
    return new Response(JSON.stringify(body), {
      status,
      headers: { 'content-type': 'application/json' },
    });
  }

  it('request() calls GET /oauth/remote/request with bearer + query', async () => {
    const fetch_ = vi.fn().mockResolvedValue(jsonResponse({ screen: 'sign-in', client: { id: 'c', name: null, uri: null, logo_uri: null, trusted: false }, scopes: [], login_hint: null, prompt: null, sessions: [], state: 'ns' }));
    const c = createPdsClient(base, token, fetch_);
    const payload = await c.request({ rqid: 'r1', state: 's1', device_id: 'd1' });
    expect(payload.screen).toBe('sign-in');
    const [url, init] = fetch_.mock.calls[0];
    expect(url).toBe(`${base}/oauth/remote/request?rqid=r1&state=s1&device_id=d1`);
    expect(init.method).toBe('GET');
    expect((init.headers as Record<string, string>).Authorization).toBe(`Bearer ${token}`);
  });

  it('signIn() POSTs JSON to /oauth/remote/sign-in', async () => {
    const fetch_ = vi.fn().mockResolvedValue(jsonResponse({ screen: 'consent', client: { id: 'c', name: null, uri: null, logo_uri: null, trusted: false }, scopes: ['atproto'], login_hint: null, prompt: null, sessions: [], state: 'ns2' }));
    const c = createPdsClient(base, token, fetch_);
    await c.signIn({ rqid: 'r1', state: 's1', device_id: 'd1', identifier: 'alice', password: 'pw' });
    const [url, init] = fetch_.mock.calls[0];
    expect(url).toBe(`${base}/oauth/remote/sign-in`);
    expect(init.method).toBe('POST');
    expect(JSON.parse(init.body as string)).toMatchObject({ identifier: 'alice', password: 'pw' });
  });

  it('select, createAccount, accept, reject hit their endpoints', async () => {
    const fetch_ = vi.fn().mockImplementation(() => jsonResponse({ screen: 'consent', client: { id: 'c', name: null, uri: null, logo_uri: null, trusted: false }, scopes: [], login_hint: null, prompt: null, sessions: [], state: 'ns' }));
    const c = createPdsClient(base, token, fetch_);
    await c.select({ rqid: 'r', state: 's', device_id: 'd', did: 'did:plc:x' });
    expect(fetch_.mock.calls[0][0]).toBe(`${base}/oauth/remote/select`);
    await c.createAccount({ rqid: 'r', state: 's', device_id: 'd', handle: 'a', email: 'a@e', password: 'pw' });
    expect(fetch_.mock.calls[1][0]).toBe(`${base}/oauth/remote/create-account`);
    await c.accept({ rqid: 'r', state: 's', device_id: 'd', did: 'did:plc:x' });
    expect(fetch_.mock.calls[2][0]).toBe(`${base}/oauth/remote/accept`);
    await c.reject({ rqid: 'r', state: 's', device_id: 'd' });
    expect(fetch_.mock.calls[3][0]).toBe(`${base}/oauth/remote/reject`);
  });

  it('passkeyStart/Finish/RegisterStart/RegisterFinish hit their endpoints', async () => {
    const fetch_ = vi.fn().mockImplementation(() => jsonResponse({ screen: 'consent', client: { id: 'c', name: null, uri: null, logo_uri: null, trusted: false }, scopes: [], login_hint: null, prompt: null, sessions: [], state: 'ns' }));
    const c = createPdsClient(base, token, fetch_);
    await c.passkeyStart({ rqid: 'r', state: 's', device_id: 'd', mode: 'entryway' });
    expect(fetch_.mock.calls[0][0]).toBe(`${base}/oauth/remote/passkey/start`);
    await c.passkeyFinish({ rqid: 'r', state: 's', device_id: 'd', credential_id: 'x', client_data_json: 'a', authenticator_data: 'b', signature: 'c' });
    expect(fetch_.mock.calls[1][0]).toBe(`${base}/oauth/remote/passkey/finish`);
    await c.passkeyRegisterStart({ rqid: 'r', state: 's', device_id: 'd', did: 'did:plc:x' });
    expect(fetch_.mock.calls[2][0]).toBe(`${base}/oauth/remote/passkey/register/start`);
    await c.passkeyRegisterFinish({ rqid: 'r', state: 's', device_id: 'd', did: 'did:plc:x', credential_id: 'x', attestation_object: 'o', client_data_json: 'a' });
    expect(fetch_.mock.calls[3][0]).toBe(`${base}/oauth/remote/passkey/register/finish`);
  });

  it('accept/reject return RedirectPayload', async () => {
    const fetch_ = vi.fn().mockResolvedValue(jsonResponse({ redirect_url: 'https://client/cb?code=1' }));
    const c = createPdsClient(base, token, fetch_);
    const r = await c.accept({ rqid: 'r', state: 's', device_id: 'd', did: 'did:plc:x' });
    expect(r.redirect_url).toBe('https://client/cb?code=1');
  });

  it('repoStatus() is unauthenticated GET /xrpc/com.atproto.sync.getRepoStatus', async () => {
    const fetch_ = vi.fn().mockResolvedValue(jsonResponse({ did: 'did:plc:x', active: true, rev: 'rev' }));
    const c = createPdsClient(base, token, fetch_);
    const r = await c.repoStatus('did:plc:x');
    expect(r).toEqual({ did: 'did:plc:x', active: true, rev: 'rev' });
    const [url, init] = fetch_.mock.calls[0];
    expect(url).toBe(`${base}/xrpc/com.atproto.sync.getRepoStatus?did=did%3Aplc%3Ax`);
    expect(init.headers.Authorization).toBeUndefined();
  });

  it('401 throws InvalidStateError', async () => {
    const fetch_ = vi.fn().mockResolvedValue(new Response('invalid state', { status: 401 }));
    const c = createPdsClient(base, token, fetch_);
    await expect(c.request({ rqid: 'r', state: 's', device_id: 'd' })).rejects.toBeInstanceOf(InvalidStateError);
  });

  it('{error,error_description} body throws PdsError', async () => {
    const fetch_ = vi.fn().mockResolvedValue(jsonResponse({ error: 'invalid_request', error_description: 'bad', client: { id: 'c', name: null, uri: null, logo_uri: null, trusted: false }, scopes: [], login_hint: null, prompt: null, sessions: [], state: 's', screen: 'error' }, 400));
    const c = createPdsClient(base, token, fetch_);
    await expect(c.signIn({ rqid: 'r', state: 's', device_id: 'd', identifier: 'a', password: 'b' })).rejects.toBeInstanceOf(PdsError);
  });
});
