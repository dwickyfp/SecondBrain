import { execFileSync } from 'node:child_process';
import { readdirSync, readFileSync } from 'node:fs';
import { writeFile } from 'node:fs/promises';

const [pidArg, output, intervalArg = '500', countArg = '20'] = process.argv.slice(2);
const rootPid = Number(pidArg);
const sampleIntervalMs = Number(intervalArg);
const requestedSampleCount = Number(countArg);
if (!Number.isInteger(rootPid) || rootPid <= 0 || !output || !Number.isInteger(sampleIntervalMs) || sampleIntervalMs < 100 || !Number.isInteger(requestedSampleCount) || requestedSampleCount < 5) {
  throw new Error('usage: node sample-process-tree-rss.mjs <root-pid> <output-json> [interval-ms>=100] [sample-count>=5]');
}

const platform = { darwin: 'macos', linux: 'linux', win32: 'windows' }[process.platform];
if (!platform) throw new Error(`unsupported RSS sampling platform: ${process.platform}`);

function unixProcesses() {
  if (process.platform === 'darwin') {
    return execFileSync('ps', ['-axo', 'pid=,ppid=,rss=,command='], { encoding: 'utf8' }).trim().split('\n').map((line) => {
      const match = /^\s*(\d+)\s+(\d+)\s+(\d+)\s+(.+)$/.exec(line);
      if (!match) return null;
      return { pid: Number(match[1]), ppid: Number(match[2]), rssBytes: Number(match[3]) * 1024, command: match[4] };
    }).filter((item) => item && Number.isInteger(item.pid) && Number.isInteger(item.ppid) && Number.isFinite(item.rssBytes));
  }
  return readdirSync('/proc', { withFileTypes: true }).filter((entry) => entry.isDirectory() && /^\d+$/.test(entry.name)).flatMap((entry) => {
    try {
      const status = readFileSync(`/proc/${entry.name}/status`, 'utf8');
      const ppid = Number(/^PPid:\s+(\d+)/m.exec(status)?.[1]);
      const rssKiB = Number(/^VmRSS:\s+(\d+)\s+kB/m.exec(status)?.[1]);
      return Number.isFinite(rssKiB) ? [{ pid: Number(entry.name), ppid, rssBytes: rssKiB * 1024 }] : [];
    } catch {
      return [];
    }
  });
}

function windowsProcesses() {
  const command = 'Get-CimInstance Win32_Process | ForEach-Object { "$($_.ProcessId) $($_.ParentProcessId) $($_.WorkingSetSize)" }';
  return execFileSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', command], { encoding: 'utf8' }).trim().split(/\r?\n/).map((line) => {
    const [pid, ppid, rssBytes] = line.trim().split(/\s+/).map(Number);
    return { pid, ppid, rssBytes };
  }).filter(({ pid, ppid, rssBytes }) => Number.isInteger(pid) && Number.isInteger(ppid) && Number.isFinite(rssBytes));
}

function sample() {
  const processes = process.platform === 'win32' ? windowsProcesses() : unixProcesses();
  const included = new Set([rootPid]);
  if (process.platform === 'darwin') {
    const rootCommand = processes.find(({ pid }) => pid === rootPid)?.command;
    const rootName = rootCommand?.split(/\s+/)[0].split('/').at(-1);
    const appContents = rootCommand?.match(/^(.*\.app\/Contents)\/MacOS\//)?.[1];
    let bundleIdentifier = null;
    if (appContents) {
      try {
        bundleIdentifier = execFileSync('plutil', ['-extract', 'CFBundleIdentifier', 'raw', '-o', '-', `${appContents}/Info.plist`], { encoding: 'utf8' }).trim();
      } catch {
        // Parent traversal still accounts for ordinary WebKit descendants.
      }
    }
    if (rootName || bundleIdentifier) {
      for (const processInfo of processes) {
        if (processInfo.command?.includes('com.apple.WebKit.') && ((rootName && processInfo.command.includes(rootName)) || (bundleIdentifier && processInfo.command.includes(bundleIdentifier)))) included.add(processInfo.pid);
      }
    }
  }
  let changed = true;
  while (changed) {
    changed = false;
    for (const processInfo of processes) {
      if (included.has(processInfo.ppid) && !included.has(processInfo.pid)) {
        included.add(processInfo.pid);
        changed = true;
      }
    }
  }
  const members = processes.filter(({ pid }) => included.has(pid));
  if (!members.some(({ pid }) => pid === rootPid)) throw new Error(`root process ${rootPid} is not running`);
  return {
    rssBytes: members.reduce((total, item) => total + item.rssBytes, 0),
    processCount: members.length
  };
}

const sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));
const samples = [];
for (let index = 0; index < requestedSampleCount; index += 1) {
  if (index > 0) await sleep(sampleIntervalMs);
  samples.push(sample());
}
if (samples.length === 0 || samples.some(({ rssBytes }) => !Number.isSafeInteger(rssBytes) || rssBytes <= 0)) throw new Error('native process-tree RSS sampling produced no valid samples');

const sorted = samples.map(({ rssBytes }) => rssBytes).sort((a, b) => a - b);
const percentile = (value) => sorted[Math.ceil((value / 100) * sorted.length) - 1];
const receipt = {
  schema: 'secondbrain.native-process-tree-rss.v1',
  version: 1,
  platform,
  root_pid: rootPid,
  sample_interval_ms: sampleIntervalMs,
  sample_count: samples.length,
  methodology: process.platform === 'darwin'
    ? 'parent and all recursively discovered descendants from ps -axo pid,ppid,rss,command plus reparented com.apple.WebKit XPC processes carrying the root executable name or bundle identifier derived from Info.plist; RSS KiB converted to bytes'
    : process.platform === 'linux'
      ? 'parent and all recursively discovered descendants from /proc status PPid and VmRSS; VmRSS KiB converted to bytes; Xvfb is excluded from the application tree'
      : 'parent and all recursively discovered descendants from Win32_Process ParentProcessId and WorkingSetSize via Windows PowerShell',
  raw_rss_bytes: samples.map(({ rssBytes }) => rssBytes),
  raw_process_counts: samples.map(({ processCount }) => processCount),
  p50_bytes: percentile(50),
  p95_bytes: percentile(95),
  p99_bytes: percentile(99)
};
await writeFile(output, `${JSON.stringify(receipt, null, 2)}\n`);
process.stdout.write(`native process-tree RSS p95 ${receipt.p95_bytes} bytes across ${receipt.sample_count} samples\n`);
