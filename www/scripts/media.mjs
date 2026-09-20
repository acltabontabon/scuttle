#!/usr/bin/env node
// The demo media lives in docs/media, where the README and the release notes
// already point at it. The site uses the same files rather than a second copy
// that can fall out of date, so they are brought in at build time instead of
// being committed twice.

import { cpSync, existsSync, mkdirSync, readdirSync, rmSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const www = join(dirname(fileURLToPath(import.meta.url)), '..');
const from = join(www, '../docs/media');
const to = join(www, 'public/media');

// Cleared first, not merged into: a copy that only ever adds files keeps
// serving media that has since been deleted from docs/media, which is how a
// build ends up shipping a screenshot nobody can find the source of.
rmSync(to, { recursive: true, force: true });
mkdirSync(to, { recursive: true });

if (!existsSync(from)) {
  console.warn('No docs/media yet — the site will fall back to its drawn illustrations.');
  process.exit(0);
}

cpSync(from, to, { recursive: true });

const sizes = readdirSync(to)
  .filter((name) => statSync(join(to, name)).isFile())
  .map((name) => `  ${name} (${(statSync(join(to, name)).size / 1024 / 1024).toFixed(2)} MB)`);
console.log(sizes.length ? `Copied from docs/media:\n${sizes.join('\n')}` : 'docs/media is empty.');
