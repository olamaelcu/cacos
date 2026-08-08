import { describe, it, expect } from 'vitest';
import { render } from '@testing-library/svelte';
import SignIn from '../../src/lib/components/screens/SignIn.svelte';
import Consent from '../../src/lib/components/screens/Consent.svelte';
import CreateAccount from '../../src/lib/components/screens/CreateAccount.svelte';
import ErrorScreen from '../../src/lib/components/screens/Error.svelte';

const baseProps = {
  rqid: 'r', state: 's', deviceId: 'd',
  client: { id: 'c', name: 'Example', uri: null, logo_uri: null, trusted: false },
};

describe('Screens render', () => {
  it('SignIn renders identifier + password fields', () => {
    const { getByLabelText } = render(SignIn, { props: { ...baseProps, loginHint: null, error: null } });
    expect(getByLabelText(/identifier/i)).toBeTruthy();
    expect(getByLabelText(/password/i)).toBeTruthy();
  });

  it('Consent renders client + Allow/Deny', () => {
    const { getByText } = render(Consent, { props: { ...baseProps, scopes: ['atproto'], session: { did: 'did:plc:x', handle: 'a', email: null } } });
    expect(getByText(/Example/)).toBeTruthy();
    expect(getByText(/Allow/i)).toBeTruthy();
    expect(getByText(/Deny/i)).toBeTruthy();
  });

  it('CreateAccount renders handle/email/password/invite fields', () => {
    const { getByLabelText } = render(CreateAccount, { props: { ...baseProps, error: null, error_description: null } });
    expect(getByLabelText(/handle/i)).toBeTruthy();
    expect(getByLabelText(/email/i)).toBeTruthy();
    expect(getByLabelText(/password/i)).toBeTruthy();
    expect(getByLabelText(/invite/i)).toBeTruthy();
  });

  it('Error surfaces error_description', () => {
    const { getByText } = render(ErrorScreen, { props: { ...baseProps, error: 'invalid_request', error_description: 'bad handle' } });
    expect(getByText(/bad handle/)).toBeTruthy();
  });
});
