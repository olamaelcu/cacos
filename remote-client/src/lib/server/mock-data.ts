// src/lib/server/mock-data.ts
// Demo data for `?mock=<screen>` mode. Lets the RemoteClient screens be
// walked end-to-end without a running cacos PDS. Only loaded in dev.
import type { ClientInfo, PagePayload, Screen, SessionInfo } from './pds/types';

export const MOCK_RQID = 'mock-rqid';
export const MOCK_STATE = 'mock-state';

const MOCK_REDIRECT_ACCEPT = '/mock-client?code=mock-code&state=' + MOCK_STATE;
const MOCK_REDIRECT_REJECT =
  '/mock-client?error=access_denied&error_description=The+user+denied+the+request&state=' + MOCK_STATE;

export const MOCK_SCREENS: readonly Screen[] = ['sign-in', 'select', 'consent', 'create', 'error'] as const;

export function isMockScreen(value: string): value is Screen {
  return (MOCK_SCREENS as readonly string[]).includes(value);
}

export function mockClientInfo(): ClientInfo {
  return {
    id: 'mock-client',
    name: 'Mock Client App',
    uri: 'https://mock-client.example',
    logo_uri: null,
    trusted: true,
  };
}

export function mockSession(): SessionInfo {
  return {
    did: 'did:plc:mockaliceabcdef1234567890',
    handle: 'alice.mock',
    email: 'alice@example.test',
  };
}

export function mockAcceptRedirect(): string {
  return MOCK_REDIRECT_ACCEPT;
}

export function mockRejectRedirect(): string {
  return MOCK_REDIRECT_REJECT;
}

export function mockPayload(screen: Screen): PagePayload {
  const client = mockClientInfo();
  const session = mockSession();
  const state = MOCK_STATE;
  switch (screen) {
    case 'sign-in':
      return {
        screen, client,
        scopes: [], login_hint: 'alice.mock', prompt: null,
        sessions: [], state,
      };
    case 'select':
      return {
        screen, client,
        scopes: [], login_hint: null, prompt: null,
        sessions: [
          session,
          { did: 'did:plc:mockbobcdefghij1234567890', handle: 'bob.mock', email: null },
        ],
        state,
      };
    case 'consent':
      return {
        screen, client,
        scopes: ['atproto', 'transition:generic', 'transition:email'],
        login_hint: null, prompt: null,
        sessions: [session],
        state,
      };
    case 'create':
      return {
        screen, client,
        scopes: [], login_hint: null, prompt: 'create',
        sessions: [], state,
      };
    case 'error':
      return {
        screen, client,
        scopes: [], login_hint: null, prompt: null,
        sessions: [], state,
        error: 'invalid_request',
        error_description: 'Demo error state — this is what the error screen looks like.',
      };
  }
}
