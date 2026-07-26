import { readFile } from 'node:fs/promises';
import { validateReadiness } from './package-contract.mjs';

const [path, commit, fixture] = process.argv.slice(2);
const marker = JSON.parse(await readFile(path, 'utf8'));
validateReadiness(marker, commit, fixture);
process.stdout.write(`${JSON.stringify(marker)}\n`);
