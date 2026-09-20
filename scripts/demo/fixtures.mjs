#!/usr/bin/env node
// Builds the pretend home directory the demo recording rummages through.
//
// Every name in here is invented. Nothing is copied from, linked to, or
// derived from the machine running this — the whole tree is written from
// scratch under one directory you pass in, and Scuttle is then pointed at
// that directory by setting HOME, so its scan roots, its database and its
// drawer are all inside it. The recording cannot reach a real file because
// the running application has never been told where one is.
//
//   node scripts/demo/fixtures.mjs <directory>
//
// The shape of the tree is chosen so that a rummage finds one of each kind of
// thing worth showing: software that is no longer installed, a burst of
// screenshots, installers that already did their job, and copies of the same
// file. The sizes are small — a few megabytes total — but the *reported*
// sizes come from the files themselves, so nothing on screen is invented
// either. It reads as a tidy demonstration rather than a frightening one,
// which is the honest impression: most of what a rummage finds is small.

import { mkdirSync, rmSync, writeFileSync, utimesSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';

const MB = 1024 * 1024;
const DAY = 86_400;

const target = process.argv[2];
if (!target) {
  console.error('Usage: node scripts/demo/fixtures.mjs <directory>');
  process.exit(1);
}
const home = resolve(target);

// A refusal rather than a surprise: this function deletes what it is pointed
// at, and being pointed at a real home directory is the one mistake worth
// making impossible.
const real = process.env.HOME ?? '';
if (home === real || (real && home.split('/').length <= real.split('/').length && real.startsWith(home))) {
  console.error(`Refusing to build fixtures over ${home}. Pick a directory of its own.`);
  process.exit(1);
}

rmSync(home, { recursive: true, force: true });

/** Deterministic bytes, so the same fixture always hashes the same way. */
function filler(size, seed) {
  const bytes = Buffer.allocUnsafe(size);
  let x = seed >>> 0;
  for (let i = 0; i < size; i++) {
    x = (x * 1664525 + 1013904223) >>> 0;
    bytes[i] = x & 0xff;
  }
  return bytes;
}

function file(relative, { size = 4096, days = 30, seed = 1, bytes = null } = {}) {
  const path = join(home, relative);
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, bytes ?? filler(size, seed));
  // Ageing is what most of the evidence is made of: "nobody has opened this
  // in eight months" is a fact about the file, and the demo should show the
  // real one rather than a placeholder.
  const when = Date.now() / 1000 - days * DAY;
  utimesSync(path, when, when);
  return path;
}

// --- Software that is no longer installed -----------------------------------
// Application Support folders whose bundle identifiers match nothing on this
// machine, because these products do not exist. That is exactly what a
// genuine leftover looks like, and it is why the detector finds them.
for (const n of [0, 1, 2, 3, 4, 5, 6, 7]) {
  file(`Library/Application Support/com.harborlight.tidepool/Caches/shader-${n}.bin`, {
    size: 3 * MB,
    days: 260,
    seed: 31 + n,
  });
}
file('Library/Application Support/com.harborlight.tidepool/logs/session.log', {
  size: 512 * 1024,
  days: 260,
  seed: 77,
});

for (const n of [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]) {
  file(`Library/Application Support/com.pennyfarthing.almanac/Cache/page-${n}.dat`, {
    size: 2 * MB,
    days: 410,
    seed: 90 + n,
  });
}

// --- Screenshots --------------------------------------------------------------
// The interesting finding is never one capture. It is the eleven attempts at
// the same thing on the same afternoon, none of which were looked at again.
const afternoon = 195;
for (let n = 1; n <= 11; n++) {
  file(`Desktop/Screenshot 2026-02-14 at 15.${String(20 + n).padStart(2, '0')}.41.png`, {
    size: 1200 * 1024 + n * 4096,
    days: afternoon,
    seed: 400 + n,
  });
}
file('Pictures/Screenshots/Screenshot 2025-11-02 at 09.14.07.png', {
  size: 980 * 1024,
  days: 320,
  seed: 512,
});

// --- Installers that already did their job -------------------------------------
// Five copies of the same download, which is what a browser does when you
// click the link again rather than look in Downloads.
file('Downloads/Marlinspike.dmg', { size: 9 * MB, days: 220, seed: 7 });
for (let n = 1; n <= 4; n++) {
  file(`Downloads/Marlinspike (${n}).dmg`, { size: 9 * MB, days: 230 + n * 9, seed: 7 });
}
file('Downloads/Quillfeather-2.4.1.dmg', { size: 6 * MB, days: 150, seed: 21 });

// --- The same bytes, more than once --------------------------------------------
const brief = filler(1500 * 1024, 900);
file('Downloads/quarterly-brief.pdf', { days: 210, bytes: brief });
file('Downloads/archive/quarterly-brief.pdf', { days: 380, bytes: brief });
file('Desktop/quarterly-brief (final).pdf', { days: 60, bytes: brief });

// --- And a tidy corner, so the demo is not all findings -------------------------
// A rummage that flagged everything would be a rummage worth distrusting. These
// sit in folders Scuttle does look at, and it leaves every one of them alone.
file('Downloads/harbour-survey-2026.pdf', { size: 700 * 1024, days: 6, seed: 3 });
file('Desktop/Letter to the harbourmaster.md', { size: 12 * 1024, days: 4, seed: 4 });
file('Desktop/Sea shanties.txt', { size: 8 * 1024, days: 1, seed: 5 });
file('Desktop/todo.txt', { size: 400, days: 0, seed: 9 });

mkdirSync(join(home, 'Library/Application Support'), { recursive: true });
console.log(`Fixture home built at ${home}`);
