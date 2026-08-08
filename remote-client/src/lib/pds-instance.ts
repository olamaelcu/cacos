// src/lib/pds-instance.ts
import { createPdsClient, type PdsClient } from '$lib/server/pds/client';
import { loadEnv } from '$lib/server/env';

let cached: PdsClient | null = null;
export function pds(): PdsClient {
  if (cached) return cached;
  const env = loadEnv();
  cached = createPdsClient(env.PDS_URL, env.PDS_OAUTH_REMOTE_CLIENT_TOKEN);
  return cached;
}
