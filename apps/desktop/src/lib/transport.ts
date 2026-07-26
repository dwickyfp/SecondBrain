import { invoke as tauriInvoke } from '@tauri-apps/api/core';

type BrowserTransport = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

declare global {
  interface Window {
    __SECONDBRAIN_TRANSPORT__?: BrowserTransport;
  }
}

export function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (import.meta.env.VITE_E2E === '1' && typeof window !== 'undefined') {
    const transport = window.__SECONDBRAIN_TRANSPORT__;
    if (!transport) return Promise.reject(new Error('E2E browser transport was not installed'));
    return transport<T>(command, args);
  }
  return tauriInvoke<T>(command, args);
}
