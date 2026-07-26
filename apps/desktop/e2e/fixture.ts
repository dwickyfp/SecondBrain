import type { Page } from '@playwright/test';

const noteA = { noteId: '01ARZ3NDEKTSV4RRFFQ69G5FAV', path: 'welcome.md', title: 'Welcome' };
const noteB = { noteId: '01ARZ3NDEKTSV4RRFFQ69G5FAW', path: 'linked.md', title: 'Linked note' };

export async function installTransport(page: Page) {
  await page.addInitScript(({ noteA, noteB }) => {
    let source = '# Welcome\n\nBrowser fixture text.\n\n## Details\n';
    const healthy = {
      formatVersion: 1, workspace: '/fixture/vault', workspaceId: '01ARZ3NDEKTSV4RRFFQ69G5FAX', status: 'healthy',
      index: { status: 'ready', path: '/fixture/vault/.secondbrain/index.sqlite', notes: 2, links: 1, brokenLinks: 0, orphans: 1 },
      transactions: { total: 1, committed: 1, aborted: 0, pending: 0, indexRepairsOutstanding: 0, reviewsPending: 0 }, issues: []
    };
    const commands: Record<string, (args: Record<string, unknown>) => unknown> = {
      open_workspace: () => ({ root: '/fixture/vault', workspaceId: healthy.workspaceId, notes: [noteA, noteB], indexed: 2, brokenLinks: 0, orphans: 1 }),
      choose_workspace_directory: () => '/fixture/vault',
      read_note: (args) => ({ ...(args.path === noteB.path ? noteB : noteA), source: args.path === noteB.path ? '# Linked note\n\nBacklink target.\n' : source, version: 0 }),
      note_context: (args) => ({ noteId: args.noteId, outline: [{ level: 1, text: args.noteId === noteA.noteId ? 'Welcome' : 'Linked note', line: 1 }], backlinks: args.noteId === noteB.noteId ? [noteA] : [] }),
      properties_read: () => ({}),
      search_workspace: () => [{ ...noteB, snippet: 'Backlink target.' }],
      inspect_workspace: () => healthy,
      recover_workspace: () => ({ formatVersion: 1, workspace: healthy.workspace, status: 'recovered', repaired: 1, quarantined: 1, abandoned: 0, reviewRequired: 1, indexRefreshed: true, diagnostics: healthy, actions: [{ code: 'SB-INDEX-REPAIRED', action: 'index_repaired', transactionId: 'txn-1', noteId: noteA.noteId, path: noteA.path, message: 'Recovered Markdown was included in a successful index rebuild.', retryable: false, reviewRequired: false }, { code: 'SB-JOURNAL-QUARANTINED', action: 'quarantined', transactionId: 'txn-2', noteId: noteB.noteId, path: noteB.path, quarantinePath: '/fixture/vault/.secondbrain/oplog/quarantine.bin', reason: 'journal_corruption', message: 'A damaged journal suffix was preserved for manual review; note bytes were not discarded.', retryable: false, reviewRequired: true }] }),
      workspace_graph: () => ({ format: 'sb-workspace-graph-v1', nodes: [{ ...noteA, note_id: noteA.noteId, incoming_occurrences: 0, outgoing_occurrences: 1, orphan: false }, { ...noteB, note_id: noteB.noteId, incoming_occurrences: 1, outgoing_occurrences: 0, orphan: false }].map(({ noteId: _noteId, ...node }) => node), edges: [{ source_note_id: noteA.noteId, target_note_id: noteB.noteId, occurrences: 1, self_link: false }], broken_links: [], ambiguous_links: [] }),
      transaction_preview: (args) => ({ format: 'sb-transaction-preview-v1', workspace_id: healthy.workspaceId, note_id: noteA.noteId, path: args.path, actor: 'secondbrain-desktop', device: 'local-desktop', expected_hash: 'a'.repeat(64), expected_version: 0, proposed_source_hash: 'b'.repeat(64), review_required: false, summary: 'Replace edited content', operations: [{ ReplaceNode: { anchor: { path: { indices: [0] }, kind: 'Root', content_hash: 'c'.repeat(64) }, content: args.proposedSource } }] }),
      transaction_apply: (args) => { source = (args.preview as { operations: Array<{ ReplaceNode: { content: string } }> }).operations[0].ReplaceNode.content; return { transaction_id: 'txn-3', note_id: noteA.noteId, path: noteA.path, changed: true, version: 1, index_refreshed: true }; },
      import_preview: () => ({ format: 'sb-obsidian-import-preview-v1', root: '/fixture/import', fingerprint: 'fixture', alreadyInitialized: false, workspaceId: null, manifestFormatVersion: null, manifestFingerprint: null, inventory: { markdown: ['a.md'], attachments: [], obsidianConfig: [], ignored: [], unsupported: [], symlinks: [] }, parseErrors: [], duplicateIds: [], portableCollisions: [], brokenLinks: [], ambiguousLinks: [], plannedWrites: { markdown: 0, attachments: 0, obsidianConfig: 0 }, canApply: true })
    };
    window.__SECONDBRAIN_TRANSPORT__ = async <T>(command: string, args: Record<string, unknown> = {}) => {
      const handler = commands[command];
      if (!handler) throw new Error(`Fixture has no command: ${command}`);
      return handler(args) as T;
    };
  }, { noteA, noteB });
}

export async function openWorkspaceByKeyboard(page: Page) {
  await page.goto('/');
  await page.getByLabel('Workspace folder').fill('/fixture/vault');
  await page.getByLabel('Workspace folder').press('Enter');
  await page.getByRole('heading', { name: 'Welcome', level: 1 }).waitFor();
}
