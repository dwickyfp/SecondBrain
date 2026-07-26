import { readdir } from 'node:fs/promises';
import { resolve } from 'node:path';

const [rootArg, extensionArg, variable = 'SB_EXACT_INSTALLER'] = process.argv.slice(2);
if (!rootArg || !/^\.[a-z0-9]+$/i.test(extensionArg ?? '') || !/^[A-Z_][A-Z0-9_]*$/.test(variable)) {
  throw new Error('usage: node locate-artifact.mjs <bundle-directory> <.extension> [ENVIRONMENT_VARIABLE]');
}

const root = resolve(rootArg);
const matches = [];
async function visit(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  entries.sort((left, right) => left.name.localeCompare(right.name));
  for (const entry of entries) {
    const path = resolve(directory, entry.name);
    if (entry.isDirectory()) await visit(path);
    else if (entry.isFile() && entry.name.toLowerCase().endsWith(extensionArg.toLowerCase())) matches.push(path);
  }
}

await visit(root);
if (matches.length !== 1) {
  throw new Error(`expected exactly one ${extensionArg} artifact below ${root}, found ${matches.length}: ${matches.join(', ') || '(none)'}`);
}
process.stdout.write(`${variable}=${matches[0]}\n`);
