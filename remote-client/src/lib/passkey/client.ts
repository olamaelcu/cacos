// src/lib/passkey/client.ts
import { startAuthentication, startRegistration } from '@simplewebauthn/browser';
import type { PasskeyRegisterStartPayload, PasskeyStartPayload } from '$lib/server/pds/types';

export async function beginAuth(p: PasskeyStartPayload) {
  const opts = {
    challenge: p.challenge,
    rpId: p.rp_id,
    userVerification: p.user_verification,
    allowCredentials: p.allow_credentials?.map(c => ({ id: c.id, type: c.type as 'public-key' })),
  };
  return startAuthentication(opts as any);
}

export async function beginRegister(p: PasskeyRegisterStartPayload) {
  const opts = {
    challenge: p.challenge,
    rp: { id: p.rp_id, name: p.rp_id },
    user: { id: p.user_handle, name: p.user_handle, displayName: p.user_handle },
    pubKeyCredParams: p.pub_key_cred_params,
    attestation: p.attestation,
  };
  return startRegistration(opts as any);
}

export function authToFinishBody(rqid: string, state: string, deviceId: string, r: any) {
  return {
    rqid, state, device_id: deviceId,
    credential_id: r.id,
    client_data_json: r.response.clientDataJSON,
    authenticator_data: r.response.authenticatorData,
    signature: r.response.signature,
    user_handle: r.response.userHandle ?? undefined,
  };
}

export function registerToFinishBody(rqid: string, state: string, deviceId: string, did: string, r: any) {
  return {
    rqid, state, device_id: deviceId, did,
    credential_id: r.id,
    attestation_object: r.response.attestationObject,
    client_data_json: r.response.clientDataJSON,
  };
}
