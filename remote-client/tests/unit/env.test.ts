import { describe, it, expect } from 'vitest';
import { loadEnv } from '../../src/lib/server/env';

describe('loadEnv', () => {
  it('parses PDS_URL and token from env', () => {
    const env = loadEnv({ PDS_URL: 'https://pds', PDS_OAUTH_REMOTE_CLIENT_TOKEN: 't' } as any);
    expect(env.PDS_URL).toBe('https://pds');
    expect(env.PDS_OAUTH_REMOTE_CLIENT_TOKEN).toBe('t');
  });

  it('throws when PDS_URL missing', () => {
    expect(() => loadEnv({} as any)).toThrow(/PDS_URL/);
  });
});
