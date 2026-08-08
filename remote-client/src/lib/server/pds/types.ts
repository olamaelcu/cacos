// src/lib/server/pds/types.ts
// Contract types — pinned to the headless-consent and passkeys specs.
// These are the typed shape of the JSON the PDS speaks. Verification is by
// unit tests.

export type Screen = 'sign-in' | 'select' | 'consent' | 'create' | 'error';

export interface ClientInfo {
  id: string;
  name: string | null;
  uri: string | null;
  logo_uri: string | null;
  trusted: boolean;
}

export interface SessionInfo {
  did: string;
  handle: string | null;
  email: string | null;
}

export interface PagePayload {
  screen: Screen;
  client: ClientInfo;
  scopes: string[];
  login_hint: string | null;
  prompt: string | null;
  sessions: SessionInfo[];
  state: string | null;
  error?: string;
  error_description?: string;
}

export interface RedirectPayload {
  redirect_url: string;
}

export interface StateQuery {
  rqid: string;
  state: string;
  device_id: string;
}

export interface SignInBody {
  rqid: string;
  state: string;
  device_id: string;
  identifier: string;
  password: string;
}

export interface SelectBody {
  rqid: string;
  state: string;
  device_id: string;
  did: string;
}

export interface CreateAccountBody {
  rqid: string;
  state: string;
  device_id: string;
  handle: string;
  email: string;
  password: string;
  invite_code?: string;
}

export interface AcceptBody {
  rqid: string;
  state: string;
  device_id: string;
  did: string;
}

export interface RejectBody {
  rqid: string;
  state: string;
  device_id: string;
}

// Passkey contracts (per passkeys spec §"PDS API surface").
export type PasskeyMode = 'entryway' | 'handle';

export interface PasskeyStartBody extends StateQuery {
  mode: PasskeyMode;
  identifier?: string;
}

export interface PasskeyStartPayload {
  challenge: string;          // base64url
  rp_id: string;
  user_verification: 'required' | 'preferred' | 'discouraged';
  allow_credentials?: Array<{ id: string; type: 'public-key' }>;
}

export interface PasskeyFinishBody {
  rqid: string;
  state: string;
  device_id: string;
  credential_id: string;
  client_data_json: string;
  authenticator_data: string;
  signature: string;
  user_handle?: string;
}

export interface PasskeyRegisterStartBody {
  rqid: string;
  state: string;
  device_id: string;
  did: string;
}

export interface PasskeyRegisterStartPayload {
  challenge: string;
  rp_id: string;
  user_handle: string;
  pub_key_cred_params: Array<{ type: 'public-key'; alg: number }>;
  attestation: 'none' | 'packed' | 'fido-u2f';
}

export interface PasskeyRegisterFinishBody {
  rqid: string;
  state: string;
  device_id: string;
  did: string;
  credential_id: string;
  attestation_object: string;
  client_data_json: string;
}

// Public XRPC: com.atproto.sync.getRepoStatus (Plan 08 Task 24 contract).
export interface RepoStatus {
  did: string;
  active: boolean;
  status?: string | null;
  rev?: string | null;
}

// Typed errors thrown by the client.
export class InvalidStateError extends Error {
  constructor() { super('invalid state'); this.name = 'InvalidStateError'; }
}

export class PdsError extends Error {
  constructor(public error: string, public error_description: string) {
    super(`${error}: ${error_description}`);
    this.name = 'PdsError';
  }
}
