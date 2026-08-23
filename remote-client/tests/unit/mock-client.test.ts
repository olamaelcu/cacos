import { describe, it, expect } from 'vitest';
import { load } from '../../src/routes/mock-client/+page';

function makeUrl(qs: Record<string, string> = {}) {
  const u = new URL('https://localhost/mock-client');
  for (const [k, v] of Object.entries(qs)) u.searchParams.set(k, v);
  return u;
}

describe('mock-client load', () => {
  it('extracts callback params into data', async () => {
    const data = await load({
      url: makeUrl({ code: 'mock-code', state: 'mock-state' }),
    } as any);
    expect(data).toEqual({
      code: 'mock-code',
      state: 'mock-state',
      error: null,
      error_description: null,
    });
  });

  it('extracts error params', async () => {
    const data = await load({
      url: makeUrl({
        error: 'access_denied',
        error_description: 'The user denied the request',
        state: 'mock-state',
      }),
    } as any);
    expect(data).toMatchObject({
      code: null,
      state: 'mock-state',
      error: 'access_denied',
      error_description: 'The user denied the request',
    });
  });

  it('returns nulls when no params', async () => {
    const data = await load({ url: makeUrl() } as any);
    expect(data).toEqual({ code: null, state: null, error: null, error_description: null });
  });
});
