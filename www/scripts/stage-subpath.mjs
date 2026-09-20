#!/usr/bin/env node
// Serves the built site the way GitHub Pages will: under /scuttle/, not at the
// root. `base` is relative so it should not matter — and checking that it
// actually does not matter is the entire point of staging it this way rather
// than trusting `vite preview` at the root.

import { cpSync, mkdirSync, rmSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const www = join(dirname(fileURLToPath(import.meta.url)), '..');
const staged = join(www, '.preview/scuttle');

rmSync(join(www, '.preview'), { recursive: true, force: true });
mkdirSync(staged, { recursive: true });
cpSync(join(www, 'dist'), staged, { recursive: true });

console.log('Staged at .preview/scuttle — open http://localhost:4173/scuttle/');
