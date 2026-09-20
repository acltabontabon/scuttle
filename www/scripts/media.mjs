#!/usr/bin/env node
// The demo media lives in docs/media, where the README and the release notes
// already point at it. The site uses the same files rather than a second copy
// that can fall out of date, so they are brought in at build time instead of
// being committed twice.

import { cpSync, existsSync, mkdirSync, readdirSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const www = join(dirname(fileURLToPath(import.meta.url)), '..');
const from = join(www, '../docs/media');
const to = join(www, 'public/media');

if (!existsSync(from)) {
  console.warn(`No docs/media yet — the site will fall back to its drawn illustrations.`);
  mkdirSync(to, { recursive: true });
  process.exit(0);
}

mkdirSync(to, { recursive: true });
cpSync(from, to, { recursive: true });

const sizes = readdirSync(to)
  .filter((name) => statSync(join(to, name)).isFile())
  .map((name) => `  ${name} (${(statSync(join(to, name)).size / 1024 / 1024).toFixed(2)} MB)`);
console.log(sizes.length ? `Copied from docs/media:\n${sizes.join('\n')}` : 'docs/media is empty.');
