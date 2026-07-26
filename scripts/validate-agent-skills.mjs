import { access, readdir, readFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";

const root = process.cwd();
const skillsRoot = path.join(root, ".agents", "skills");
const skillNamePattern = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;

function parseFrontmatter(source, skillFile) {
  const match = source.match(/^---\n([\s\S]*?)\n---\n/);
  if (!match) throw new Error(`${skillFile}: missing YAML frontmatter`);

  const values = new Map();
  for (const line of match[1].split("\n")) {
    if (/^\s/.test(line) || !line.includes(":")) continue;
    const separator = line.indexOf(":");
    values.set(line.slice(0, separator), line.slice(separator + 1).trim());
  }
  return values;
}

const entries = await readdir(skillsRoot, { withFileTypes: true });
const skillDirectories = entries.filter((entry) => entry.isDirectory());
if (skillDirectories.length === 0) throw new Error("no repository agent skills found");

for (const directory of skillDirectories) {
  const skillFile = path.join(skillsRoot, directory.name, "SKILL.md");
  const source = await readFile(skillFile, "utf8");
  const frontmatter = parseFrontmatter(source, skillFile);
  const name = frontmatter.get("name");
  const description = frontmatter.get("description");

  if (name !== directory.name) {
    throw new Error(`${skillFile}: name must match directory ${directory.name}`);
  }
  if (!skillNamePattern.test(name)) {
    throw new Error(`${skillFile}: invalid skill name ${name}`);
  }
  if (!description || description.length > 1024) {
    throw new Error(`${skillFile}: description must contain 1-1024 characters`);
  }
  if (source.split("\n").length > 500) {
    throw new Error(`${skillFile}: SKILL.md exceeds the 500-line recommendation`);
  }

  for (const link of source.matchAll(/\[[^\]]+\]\(([^)]+)\)/g)) {
    const target = link[1].split("#", 1)[0];
    if (!target || /^[a-z]+:/i.test(target)) continue;
    await access(path.resolve(path.dirname(skillFile), target));
  }
}

console.log(`validated ${skillDirectories.length} repository agent skills`);
