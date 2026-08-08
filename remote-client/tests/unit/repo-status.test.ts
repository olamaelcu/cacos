import { describe, it, expect, vi } from 'vitest';
import { GET } from '../../src/routes/api/repo-status/+server';

describe('GET /api/repo-status', () => {
  it('returns 400 when did missing', async () => {
    const res = await GET({ url: new URL('http://x/api/repo-status'), locals: {}, request: new Request('http://x') } as any);
    expect(res.status).toBe(400);
  });

  it('proxies GET /xrpc/com.atproto.sync.getRepoStatus unauthenticated', async () => {
    const fake = vi.fn().mockResolvedValue(new Response(JSON.stringify({ did: 'did:plc:x', active: true }), { headers: { 'content-type': 'application/json' } }));
    const realFetch = globalThis.fetch; globalThis.fetch = fake as any;
    try {
      const res = await GET({ url: new URL('http://x/api/repo-status?did=did%3Aplc%3Ax'), locals: {}, request: new Request('http://x') } as any);
      expect(res.status).toBe(200);
      expect(await res.json()).toEqual({ did: 'did:plc:x', active: true });
      const [url, init] = fake.mock.calls[0];
      expect(url).toContain('/xrpc/com.atproto.sync.getRepoStatus?did=did%3Aplc%3Ax');
      expect((init?.headers as any)?.Authorization).toBeUndefined();
    } finally { globalThis.fetch = realFetch; }
  });
});