import { createHash } from 'node:crypto';
import { readdir, readFile } from 'node:fs/promises';
import { join, relative, resolve } from 'node:path';

const root = resolve(process.argv[2] ?? '../../fixtures/markdown');
const files = [];

async function collect(directory) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) await collect(path);
    else if (entry.isFile()) files.push(path);
  }
}

await collect(root);
files.sort((a, b) => a.localeCompare(b));
const hash = createHash('sha256');
for (const file of files) {
  hash.update(relative(root, file).replaceAll('\\', '/'));
  hash.update('\0');
  hash.update(await readFile(file));
  hash.update('\0');
}
process.stdout.write(`${hash.digest('hex')}\n`);
