const exactKeys = (value, expected, name) => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error(`${name} must be an object`);
  const actual = Object.keys(value).sort();
  const wanted = [...expected].sort();
  if (actual.length !== wanted.length || actual.some((key, index) => key !== wanted[index])) throw new Error(`${name} has missing or unknown fields`);
};

const sha256 = (value, name) => {
  if (!/^[a-f0-9]{64}$/.test(value ?? '')) throw new Error(`${name} must be a SHA-256`);
};

const positiveInteger = (value, name) => {
  if (!Number.isSafeInteger(value) || value <= 0) throw new Error(`${name} must be a positive integer`);
};

export function validateMemoryReceipt(receipt) {
  exactKeys(receipt, ['schema', 'version', 'platform', 'root_pid', 'sample_interval_ms', 'sample_count', 'methodology', 'raw_rss_bytes', 'raw_process_counts', 'p50_bytes', 'p95_bytes', 'p99_bytes'], 'memory receipt');
  if (receipt.schema !== 'secondbrain.native-process-tree-rss.v1' || receipt.version !== 1) throw new Error('unsupported memory receipt schema or version');
  if (!['macos', 'linux', 'windows'].includes(receipt.platform) || !receipt.methodology) throw new Error('memory receipt metadata is invalid');
  positiveInteger(receipt.root_pid, 'memory receipt root_pid');
  positiveInteger(receipt.sample_interval_ms, 'memory receipt sample_interval_ms');
  positiveInteger(receipt.sample_count, 'memory receipt sample_count');
  if (receipt.sample_count < 5 || !Array.isArray(receipt.raw_rss_bytes) || !Array.isArray(receipt.raw_process_counts) || receipt.raw_rss_bytes.length !== receipt.sample_count || receipt.raw_process_counts.length !== receipt.sample_count) throw new Error('memory receipt raw sample count is invalid');
  receipt.raw_rss_bytes.forEach((value) => positiveInteger(value, 'memory receipt RSS sample'));
  receipt.raw_process_counts.forEach((value) => positiveInteger(value, 'memory receipt process count'));
  const sorted = [...receipt.raw_rss_bytes].sort((a, b) => a - b);
  const percentile = (value) => sorted[Math.ceil((value / 100) * sorted.length) - 1];
  if (receipt.p50_bytes !== percentile(50) || receipt.p95_bytes !== percentile(95) || receipt.p99_bytes !== percentile(99)) throw new Error('memory receipt percentiles do not match raw samples');
}

export function validateReadiness(marker, commit, fixture, version = undefined) {
  exactKeys(marker, ['schema', 'version', 'commit', 'platform', 'fixture_sha256', 'diagnostics'], 'readiness marker');
  if (marker.schema !== 'secondbrain.desktop.readiness.v1') throw new Error('unexpected readiness schema');
  if (marker.commit !== commit || !/^[a-f0-9]{40}$/.test(marker.commit)) throw new Error('readiness commit does not match');
  if (marker.fixture_sha256 !== fixture) throw new Error('readiness fixture does not match');
  sha256(marker.fixture_sha256, 'readiness fixture');
  if (!marker.version || (version !== undefined && marker.version !== version) || !['macos', 'windows', 'linux'].includes(marker.platform) || marker.diagnostics !== 'frontend-page-loaded-and-backend-ready') throw new Error('readiness fields are invalid');
}

export function validateEvidence(evidence, artifactName, artifactSha256, { allowMutable = false } = {}) {
  exactKeys(evidence, ['schema', 'version', 'commit', 'platform', 'installer', 'performance', 'fixture_sha256', 'tools', 'signing', 'inputs', 'source', 'ci'], 'evidence');
  exactKeys(evidence.installer, ['name', 'kind', 'extension', 'sha256', 'smoke', 'readiness_sha256'], 'installer');
  exactKeys(evidence.tools, ['tauri_cli', 'rustc', 'node'], 'tools');
  exactKeys(evidence.signing, ['status', 'notarization_status', 'reason'], 'signing');
  exactKeys(evidence.inputs, ['root_cargo_lock_sha256', 'desktop_cargo_lock_sha256', 'npm_lock_sha256', 'budget_sha256', 'source_sha256'], 'inputs');
  exactKeys(evidence.source, ['dirty', 'diff_sha256', 'mutable'], 'source');
  exactKeys(evidence.ci, ['run_url'], 'ci');
  exactKeys(evidence.performance, ['sample_sha256', 'p95_bytes', 'budget_bytes', 'status', 'mode'], 'performance');
  if (evidence.schema !== 'secondbrain.packaging-evidence.v1' || !evidence.version) throw new Error('unsupported evidence schema or version');
  if (!/^[a-f0-9]{40}$/.test(evidence.commit ?? '')) throw new Error('evidence commit must be a full Git commit');
  sha256(evidence.fixture_sha256, 'fixture');
  for (const [key, value] of Object.entries(evidence.inputs)) sha256(value, `inputs.${key}`);
  sha256(evidence.source.diff_sha256, 'source.diff_sha256');
  sha256(evidence.installer.sha256, 'installer.sha256');
  sha256(evidence.installer.readiness_sha256, 'installer.readiness_sha256');
  sha256(evidence.performance.sample_sha256, 'performance.sample_sha256');
  positiveInteger(evidence.performance.p95_bytes, 'performance.p95_bytes');
  const contracts = {
    'macOS-DMG': ['macOS', '.dmg'],
    'Windows-MSI': ['Windows', '.msi'],
    'Linux-DEB': ['Linux', '.deb']
  };
  const contract = contracts[evidence.installer.kind];
  if (!contract || evidence.platform !== contract[0] || evidence.installer.extension !== contract[1] || !artifactName.endsWith(contract[1])) throw new Error('platform, installer kind, and extension do not agree');
  const expectedSmoke = evidence.platform === 'macOS' ? 'passed-exact-artifact-mount-and-launch' : 'passed-exact-artifact-extraction-and-launch';
  if (evidence.installer.name !== artifactName || evidence.installer.sha256 !== artifactSha256 || evidence.installer.smoke !== expectedSmoke) throw new Error('evidence does not identify the exact smoked artifact and launch method');
  if (evidence.source.dirty !== evidence.source.mutable || typeof evidence.source.dirty !== 'boolean') throw new Error('source dirty and mutable state must agree');
  if (evidence.source.mutable && !allowMutable) throw new Error('mutable local evidence is not immutable release evidence');
  if (evidence.performance.mode === 'named-reference') {
    positiveInteger(evidence.performance.budget_bytes, 'performance.budget_bytes');
    if (evidence.performance.status !== 'passed' || evidence.performance.p95_bytes >= evidence.performance.budget_bytes) throw new Error('named-reference memory evidence must strictly pass its budget');
  } else if (evidence.performance.mode === 'hosted-ci-smoke-non-reference') {
    if (evidence.performance.budget_bytes !== null || evidence.performance.status !== 'recorded-not-enforced') throw new Error('hosted CI memory evidence cannot claim an absolute budget pass');
  } else {
    throw new Error('performance mode is invalid');
  }
  if (!evidence.tools.tauri_cli.startsWith('tauri-cli ') || !evidence.tools.rustc.startsWith('rustc ') || !/^v\d+/.test(evidence.tools.node)) throw new Error('tool metadata is invalid');
  if (!/^https:\/\/github\.com\/[^/]+\/[^/]+\/actions\/runs\/\d+$/.test(evidence.ci.run_url) && !(allowMutable && evidence.source.mutable && evidence.ci.run_url === 'local')) throw new Error('CI run URL is invalid');
  const status = evidence.signing.status;
  const notarization = evidence.signing.notarization_status;
  const signingStatuses = ['unsigned', 'signed', 'not-configured', 'failed'];
  const notarizationStatuses = ['not-applicable', 'notarized', 'not-configured', 'failed'];
  if (!evidence.signing.reason || !signingStatuses.includes(status) || !notarizationStatuses.includes(notarization)) throw new Error('signing or notarization status is invalid');
  const legalMacPairs = new Set(['unsigned:not-applicable', 'signed:notarized', 'not-configured:not-configured', 'failed:failed']);
  if (evidence.platform === 'macOS' && !legalMacPairs.has(`${status}:${notarization}`)) throw new Error('macOS signing and notarization statuses contradict each other');
  if (evidence.platform !== 'macOS' && notarization !== 'not-applicable') throw new Error('non-macOS notarization must be not-applicable');
}
