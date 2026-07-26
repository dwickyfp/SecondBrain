import { invoke } from '@tauri-apps/api/core';
import packageMetadata from '../../package.json';

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
  }
}

export async function reportReleaseReadiness(): Promise<void> {
  if (typeof window === 'undefined' || !window.__TAURI_INTERNALS__) return;
  await invoke('release_readiness', { frontendVersion: packageMetadata.version });
}
