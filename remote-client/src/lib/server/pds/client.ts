// src/lib/server/pds/client.ts
import {
  type AcceptBody, type CreateAccountBody, type PagePayload, type PasskeyFinishBody,
  type PasskeyRegisterFinishBody, type PasskeyRegisterStartBody, type PasskeyRegisterStartPayload,
  type PasskeyStartBody, type PasskeyStartPayload, type RedirectPayload, type RejectBody,
  type RepoStatus, type SelectBody, type SignInBody, type StateQuery,
  InvalidStateError, PdsError,
} from './types';

export { InvalidStateError, PdsError };

export type FetchLike = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;

export interface PdsClient {
  request(q: StateQuery): Promise<PagePayload>;
  signIn(b: SignInBody): Promise<PagePayload>;
  select(b: SelectBody): Promise<PagePayload>;
  createAccount(b: CreateAccountBody): Promise<PagePayload>;
  accept(b: AcceptBody): Promise<RedirectPayload>;
  reject(b: RejectBody): Promise<RedirectPayload>;
  passkeyStart(b: PasskeyStartBody): Promise<PasskeyStartPayload>;
  passkeyFinish(b: PasskeyFinishBody): Promise<PagePayload>;
  passkeyRegisterStart(b: PasskeyRegisterStartBody): Promise<PasskeyRegisterStartPayload>;
  passkeyRegisterFinish(b: PasskeyRegisterFinishBody): Promise<PagePayload>;
  repoStatus(did: string): Promise<RepoStatus>;
}

export function createPdsClient(base: string, token: string, fetchFn: FetchLike = fetch): PdsClient {
  const auth = { Authorization: `Bearer ${token}` } as const;
  const json = (body: unknown) => ({ 'content-type': 'application/json', ...auth }) as Record<string, string>;

  async function call<T>(url: string, init: RequestInit, parse: (r: Response) => Promise<T>): Promise<T> {
    const res = await fetchFn(url, init);
    if (res.status === 401) throw new InvalidStateError();
    if (!res.ok) {
      // try to surface {error, error_description} for typed handling
      let body: any = null;
      try { body = await res.json(); } catch {}
      if (body && typeof body.error === 'string') throw new PdsError(body.error, body.error_description ?? '');
      throw new Error(`PDS ${res.status}`);
    }
    return parse(res);
  }

  async function jsonParse<T>(res: Response): Promise<T> { return res.json() as Promise<T>; }

  return {
    request(q) {
      const u = new URL(`${base}/oauth/remote/request`);
      u.searchParams.set('rqid', q.rqid); u.searchParams.set('state', q.state); u.searchParams.set('device_id', q.device_id);
      return call<PagePayload>(u.toString(), { method: 'GET', headers: auth }, jsonParse);
    },
    signIn(b) { return call<PagePayload>(`${base}/oauth/remote/sign-in`, { method: 'POST', headers: json(b), body: JSON.stringify(b) }, jsonParse); },
    select(b) { return call<PagePayload>(`${base}/oauth/remote/select`, { method: 'POST', headers: json(b), body: JSON.stringify(b) }, jsonParse); },
    createAccount(b) { return call<PagePayload>(`${base}/oauth/remote/create-account`, { method: 'POST', headers: json(b), body: JSON.stringify(b) }, jsonParse); },
    accept(b) { return call<RedirectPayload>(`${base}/oauth/remote/accept`, { method: 'POST', headers: json(b), body: JSON.stringify(b) }, jsonParse); },
    reject(b) { return call<RedirectPayload>(`${base}/oauth/remote/reject`, { method: 'POST', headers: json(b), body: JSON.stringify(b) }, jsonParse); },
    passkeyStart(b) { return call<PasskeyStartPayload>(`${base}/oauth/remote/passkey/start`, { method: 'POST', headers: json(b), body: JSON.stringify(b) }, jsonParse); },
    passkeyFinish(b) { return call<PagePayload>(`${base}/oauth/remote/passkey/finish`, { method: 'POST', headers: json(b), body: JSON.stringify(b) }, jsonParse); },
    passkeyRegisterStart(b) { return call<PasskeyRegisterStartPayload>(`${base}/oauth/remote/passkey/register/start`, { method: 'POST', headers: json(b), body: JSON.stringify(b) }, jsonParse); },
    passkeyRegisterFinish(b) { return call<PagePayload>(`${base}/oauth/remote/passkey/register/finish`, { method: 'POST', headers: json(b), body: JSON.stringify(b) }, jsonParse); },
    repoStatus(did) {
      const u = new URL(`${base}/xrpc/com.atproto.sync.getRepoStatus`);
      u.searchParams.set('did', did);
      return call<RepoStatus>(u.toString(), { method: 'GET', headers: {} }, jsonParse);
    },
  };
}
