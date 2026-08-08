import { describe, it, expect } from 'vitest';
import { ensureDeviceId } from '../../src/hooks.server';

describe('ensureDeviceId', () => {
  it('returns existing cookie value unchanged', () => {
    const got = ensureDeviceId({ get: () => 'abc' } as any);
    expect(got).toBe('abc');
  });

  it('mints and persists a new uuid when absent', () => {
    let stored: string | undefined;
    const cookies = {
      get: () => undefined,
      set: (_: string, v: string) => { stored = v; },
    };
    const id = ensureDeviceId(cookies as any);
    expect(id).toMatch(/^[0-9a-f-]{36}$/i);
    expect(stored).toBe(id);
  });
});
