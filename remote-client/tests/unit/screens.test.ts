import { describe, it, expect } from 'vitest';
import { render } from '@testing-library/svelte';
import SignIn from '../../src/lib/components/screens/SignIn.svelte';
import Consent from '../../src/lib/components/screens/Consent.svelte';
import CreateAccount from '../../src/lib/components/screens/CreateAccount.svelte';
import ErrorScreen from '../../src/lib/components/screens/Error.svelte';
import Splash from '../../src/lib/components/screens/Splash.svelte';

const baseProps = {
  rqid: 'r', state: 's', deviceId: 'd',
  client: { id: 'c', name: 'Example', uri: null, logo_uri: null, trusted: false },
};

describe('Screens render', () => {
  it('Splash renders without props', () => {
    const { getByRole, getByText } = render(Splash);
    expect(getByRole('heading', { name: /cacos RemoteClient/i })).toBeTruthy();
    expect(getByText(/Local dev/i)).toBeTruthy();
  });
  it('SignIn renders identifier + password fields', () => {
    const { container } = render(SignIn, { props: { ...baseProps, loginHint: null, error: null } });
    expect(container.querySelector('wa-input[name=identifier]')).toBeTruthy();
    expect(container.querySelector('wa-input[name=password]')).toBeTruthy();
    expect(container.querySelector('wa-input[name=identifier]')?.getAttribute('label')).toMatch(/identifier/i);
    expect(container.querySelector('wa-input[name=password]')?.getAttribute('label')).toMatch(/password/i);
  });

  it('Consent renders client + Allow/Deny', () => {
    const { container } = render(Consent, { props: { ...baseProps, scopes: ['atproto'], session: { did: 'did:plc:x', handle: 'a', email: null } } });
    expect(container.textContent).toMatch(/Example/);
    expect(container.querySelector('form[action="?/accept"]')).toBeTruthy();
    expect(container.querySelector('form[action="?/reject"]')).toBeTruthy();
  });

  it('CreateAccount renders handle/email/password/invite fields', () => {
    const { container } = render(CreateAccount, { props: { ...baseProps, error: null, error_description: null } });
    for (const name of ['handle', 'email', 'password', 'invite_code']) {
      expect(container.querySelector(`wa-input[name=${name}]`)).toBeTruthy();
    }
  });

  it('Error surfaces error_description', () => {
    const { getByText } = render(ErrorScreen, { props: { ...baseProps, error: 'invalid_request', error_description: 'bad handle' } });
    expect(getByText(/bad handle/)).toBeTruthy();
  });
});
