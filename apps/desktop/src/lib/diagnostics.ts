import { invoke } from './transport';

export interface DiagnosticIssue {
  code: string;
  message: string;
  action: string;
  path: string;
  retryable: boolean;
  reviewRequired: boolean;
}

export interface WorkspaceReportV1 {
  formatVersion: 1;
  workspace: string;
  workspaceId: string;
  status: 'healthy' | 'attention';
  index: {
    status: 'ready' | 'missing' | 'stale' | 'invalid';
    path: string;
    notes: number;
    links: number;
    brokenLinks: number;
    orphans: number;
  };
  transactions: {
    total: number;
    committed: number;
    aborted: number;
    pending: number;
    indexRepairsOutstanding: number;
    reviewsPending: number;
  };
  issues: DiagnosticIssue[];
}

export interface RecoveryActionV1 {
  code: string;
  action: 'index_repaired' | 'quarantined' | 'abandoned';
  transactionId: string;
  noteId: string;
  path: string;
  quarantinePath?: string;
  reason?: string;
  message: string;
  retryable: boolean;
  reviewRequired: boolean;
}

export interface RecoveryReportV1 {
  formatVersion: 1;
  workspace: string;
  status: 'review_required' | 'nothing_to_recover' | 'recovered';
  actions: RecoveryActionV1[];
  repaired: number;
  quarantined: number;
  abandoned: number;
  reviewRequired: number;
  indexRefreshed: boolean;
  diagnostics: WorkspaceReportV1;
}

export const inspectWorkspace = (root: string) =>
  invoke<WorkspaceReportV1>('inspect_workspace', { root });

export const recoverWorkspace = (root: string) =>
  invoke<RecoveryReportV1>('recover_workspace', { root });
