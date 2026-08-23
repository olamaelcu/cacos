import { describe, it, expect } from 'vitest';
import {
  isMockScreen, MOCK_RQID, MOCK_STATE, MOCK_SCREENS,
  mockAcceptRedirect, mockClientInfo, mockPayload, mockRejectRedirect, mockSession,
} from '../../src/lib/server/mock-data';

describe('mock-data', () => {
  it('exports stable rqid and state markers', () => {
    expect(MOCK_RQID).toBe('mock-rqid');
    expect(MOCK_STATE).toBe('mock-state');
  });

  it('isMockScreen narrows to valid Screen values', () => {
    expect(isMockScreen('sign-in')).toBe(true);
    expect(isMockScreen('select')).toBe(true);
    expect(isMockScreen('consent')).toBe(true);
    expect(isMockScreen('create')).toBe(true);
    expect(isMockScreen('error')).toBe(true);
    expect(isMockScreen('bogus')).toBe(false);
    expect(isMockScreen('')).toBe(false);
    expect(MOCK_SCREENS).toEqual(['sign-in', 'select', 'consent', 'create', 'error']);
  });

  it('mockClientInfo returns the demo client', () => {
    expect(mockClientInfo()).toEqual({
      id: 'mock-client',
      name: 'Mock Client App',
      uri: 'https://mock-client.example',
      logo_uri: null,
      trusted: true,
    });
  });

  it('mockSession returns the demo session', () => {
    expect(mockSession()).toEqual({
      did: 'did:plc:mockaliceabcdef1234567890',
      handle: 'alice.mock',
      email: 'alice@example.test',
    });
  });

  it('mockAcceptRedirect / mockRejectRedirect include mock-state', () => {
    expect(mockAcceptRedirect()).toBe('/mock-client?code=mock-code&state=mock-state');
    expect(mockRejectRedirect()).toBe(
      '/mock-client?error=access_denied&error_description=The+user+denied+the+request&state=mock-state'
    );
  });

  it('mockPayload("sign-in") seeds login hint and empty sessions', () => {
    const p = mockPayload('sign-in');
    expect(p.screen).toBe('sign-in');
    expect(p.login_hint).toBe('alice.mock');
    expect(p.sessions).toEqual([]);
    expect(p.scopes).toEqual([]);
    expect(p.state).toBe(MOCK_STATE);
  });

  it('mockPayload("select") seeds two sessions', () => {
    const p = mockPayload('select');
    expect(p.screen).toBe('select');
    expect(p.sessions).toHaveLength(2);
    expect(p.sessions[0].handle).toBe('alice.mock');
    expect(p.sessions[1].handle).toBe('bob.mock');
    expect(p.login_hint).toBeNull();
  });

  it('mockPayload("consent") seeds session + scopes', () => {
    const p = mockPayload('consent');
    expect(p.screen).toBe('consent');
    expect(p.scopes).toEqual(['atproto', 'transition:generic', 'transition:email']);
    expect(p.sessions).toHaveLength(1);
    expect(p.sessions[0].did).toBe('did:plc:mockaliceabcdef1234567890');
  });

  it('mockPayload("create") sets prompt=create', () => {
    const p = mockPayload('create');
    expect(p.screen).toBe('create');
    expect(p.prompt).toBe('create');
  });

  it('mockPayload("error") seeds error + description', () => {
    const p = mockPayload('error');
    expect(p.screen).toBe('error');
    expect(p.error).toBe('invalid_request');
    expect(p.error_description).toMatch(/Demo error/);
  });

  it('every payload carries the demo client and mock-state', () => {
    for (const s of MOCK_SCREENS) {
      const p = mockPayload(s);
      expect(p.client.id).toBe('mock-client');
      expect(p.state).toBe(MOCK_STATE);
    }
  });
});
