import { afterEach } from 'vitest';
import { cleanup } from '@testing-library/svelte';

process.env.PDS_URL ??= 'https://pds.test';
process.env.PDS_OAUTH_REMOTE_CLIENT_TOKEN ??= 'test-token';

afterEach(() => {
  cleanup();
});
