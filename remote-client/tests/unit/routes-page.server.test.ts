import { describe, it, expect, vi } from 'vitest';
import { actions, load } from '../../src/routes/+page.server';
import { MOCK_RQID, MOCK_STATE } from '../../src/lib/server/mock-data';

function makeEvent(qs: string, fields: Record<string, string> = {}) {
  const body = new URLSearchParams(fields).toString();
  const request = new Request(`https://localhost/${qs}`, {
    method: 'POST',
    body,
    headers: { 'content-type': 'application/x-www-form-urlencoded' },
  });
  return { request, locals: { deviceId: 'd' }, url: new URL(`https://localhost/${qs}`), cookies: {} } as any;
}

function makeLoadEvent(qs: string) {
  return { url: new URL(`https://localhost/${qs}`), locals: { deviceId: 'd' } } as any;
}

describe('mock-mode load', () => {
  it('?mock=sign-in returns synthesized payload without calling PDS', async () => {
    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockImplementation(() => {
      throw new Error('fetch should not be called in mock mode');
    });
    try {
      const data = await load(makeLoadEvent('?mock=sign-in')) as any;
      expect(data.rqid).toBe(MOCK_RQID);
      expect(data.state).toBe(MOCK_STATE);
      expect(data.deviceId).toBe('d');
      expect(data.payload.screen).toBe('sign-in');
      expect(data.payload.login_hint).toBe('alice.mock');
      expect(data.payload.client.id).toBe('mock-client');
    } finally {
      fetchSpy.mockRestore();
    }
  });

  it.each(['sign-in', 'select', 'consent', 'create', 'error'] as const)(
    '?mock=%s returns a payload with screen=%s',
    async (screen) => {
      const data = await load(makeLoadEvent(`?mock=${screen}`)) as any;
      expect(data.payload.screen).toBe(screen);
      expect(data.payload.state).toBe(MOCK_STATE);
    }
  );

  it('?mock=bogus falls through to the real flow (no PDS call would happen either)', async () => {
    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response('{}'));
    try {
      const data = await load(makeLoadEvent('?mock=bogus')) as any;
      expect(data.splash).toBe(true);
    } finally {
      fetchSpy.mockRestore();
    }
  });
});

describe('mock-mode actions', () => {
  it('signIn redirects to /?mock=consent when state=mock-state', async () => {
    try {
      await actions.signIn(makeEvent('?mock=sign-in', {
        rqid: MOCK_RQID, state: MOCK_STATE, device_id: 'd',
        identifier: 'alice', password: 'pw',
      }));
      expect.fail('expected redirect');
    } catch (e: any) {
      expect(e.status).toBe(302);
      expect(e.location).toBe('/?mock=consent');
    }
  });

  it('select redirects to /?mock=consent when state=mock-state', async () => {
    try {
      await actions.select(makeEvent('?mock=select', {
        rqid: MOCK_RQID, state: MOCK_STATE, device_id: 'd',
        did: 'did:plc:mockaliceabcdef1234567890',
      }));
      expect.fail('expected redirect');
    } catch (e: any) {
      expect(e.status).toBe(302);
      expect(e.location).toBe('/?mock=consent');
    }
  });

  it('createAccount redirects to /?mock=consent when state=mock-state', async () => {
    try {
      await actions.createAccount(makeEvent('?mock=create', {
        rqid: MOCK_RQID, state: MOCK_STATE, device_id: 'd',
        handle: 'alice', email: 'a@e.test', password: 'pw',
      }));
      expect.fail('expected redirect');
    } catch (e: any) {
      expect(e.status).toBe(302);
      expect(e.location).toBe('/?mock=consent');
    }
  });

  it('accept redirects to /mock-client with code when state=mock-state', async () => {
    try {
      await actions.accept(makeEvent('?mock=consent', {
        rqid: MOCK_RQID, state: MOCK_STATE, device_id: 'd',
        did: 'did:plc:mockaliceabcdef1234567890',
      }));
      expect.fail('expected redirect');
    } catch (e: any) {
      expect(e.status).toBe(302);
      expect(e.location).toBe('/mock-client?code=mock-code&state=mock-state');
    }
  });

  it('reject redirects to /mock-client with error when state=mock-state', async () => {
    try {
      await actions.reject(makeEvent('?mock=consent', {
        rqid: MOCK_RQID, state: MOCK_STATE, device_id: 'd',
      }));
      expect.fail('expected redirect');
    } catch (e: any) {
      expect(e.status).toBe(302);
      expect(e.location).toBe('/mock-client?error=access_denied&error_description=The+user+denied+the+request&state=mock-state');
    }
  });
});
