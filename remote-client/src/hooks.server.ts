// src/hooks.server.ts
import { randomUUID } from 'node:crypto';
import type { Handle } from '@sveltejs/kit';

export const DEVICE_COOKIE = 'rc_device_id';

export function ensureDeviceId(cookies: { get: (k: string) => string | undefined; set: (k: string, v: string, opts?: any) => void }, key = DEVICE_COOKIE): string {
  const existing = cookies.get(key);
  if (existing) return existing;
  const id = randomUUID();
  cookies.set(key, id, {
    path: '/',
    httpOnly: true,
    sameSite: 'lax',
    secure: true,
    maxAge: 60 * 60 * 24 * 365,
  });
  return id;
}

export const handle: Handle = async ({ event, resolve }) => {
  event.locals.deviceId = ensureDeviceId(event.cookies);
  return resolve(event);
};
