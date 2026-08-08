// src/lib/server/env.ts
export interface ServerEnv {
  PDS_URL: string;
  PDS_OAUTH_REMOTE_CLIENT_TOKEN: string;
}

export function loadEnv(src: NodeJS.ProcessEnv = process.env): ServerEnv {
  const PDS_URL = src.PDS_URL?.trim();
  const PDS_OAUTH_REMOTE_CLIENT_TOKEN = src.PDS_OAUTH_REMOTE_CLIENT_TOKEN?.trim();
  if (!PDS_URL) throw new Error('PDS_URL is required');
  if (!PDS_OAUTH_REMOTE_CLIENT_TOKEN) throw new Error('PDS_OAUTH_REMOTE_CLIENT_TOKEN is required');
  return { PDS_URL, PDS_OAUTH_REMOTE_CLIENT_TOKEN };
}
