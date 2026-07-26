import { open } from '@tauri-apps/plugin-dialog';
import { invoke } from './transport';

export async function chooseWorkspaceDirectory(): Promise<string | null> {
  if (import.meta.env.VITE_E2E === '1') {
    return invoke<string | null>('choose_workspace_directory');
  }

  const selected = await open({ directory: true, multiple: false });
  return typeof selected === 'string' ? selected : null;
}
