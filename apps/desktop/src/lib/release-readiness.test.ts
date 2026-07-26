import { beforeEach, describe, expect, it, vi } from 'vitest';
import { reportReleaseReadiness } from './release-readiness';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

describe('release readiness handshake', () => {
  beforeEach(() => {
    invoke.mockReset();
    delete window.__TAURI_INTERNALS__;
  });

  it('does not call the backend in browser tests', async () => {
    await reportReleaseReadiness();
    expect(invoke).not.toHaveBeenCalled();
  });

  it('sends the production frontend version to the Tauri backend', async () => {
    window.__TAURI_INTERNALS__ = {};
    invoke.mockResolvedValue(true);
    await reportReleaseReadiness();
    expect(invoke).toHaveBeenCalledWith('release_readiness', { frontendVersion: '0.1.0' });
  });
});
