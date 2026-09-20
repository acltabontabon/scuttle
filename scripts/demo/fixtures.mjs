#!/usr/bin/env node
// Builds the pretend home directory the demo recording rummages through.
//
// Every name in here is invented and every byte is generated. Nothing is
// copied from, linked to, or derived from the machine running this — the tree
// is written from scratch under one directory you pass in, and Scuttle is then
// pointed at that directory by setting HOME, so its scan roots, its database
// and its drawer all live inside it. The application in the recording cannot
// reach a real file because it has never been told where one is.
//
//   node scripts/demo/fixtures.mjs <directory>
//
// It is shaped and sized like a machine somebody actually uses: a few hundred
// thousand files, installers that are tens of megabytes because installers
// are, screenshots the size a Retina capture really is, and a leftover cache
// with the thousands of small blobs a real one has. That costs about a minute
// and a couple of gigabytes of temporary space, and it buys two things — a
// rummage you can watch happen, and numbers on screen that are worth
// believing, because they were measured rather than written.
//
//   SCUTTLE_DEMO_BULK=50000   fewer blobs, a faster and less convincing scan

import { mkdirSync, rmSync, writeFileSync, utimesSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';

const KB = 1024;
const MB = 1024 * KB;
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
if (home === real || (real && real.startsWith(home))) {
  console.error(`Refusing to build fixtures over ${home}. Give it a directory of its own.`);
  process.exit(1);
}

rmSync(home, { recursive: true, force: true });

/**
 * Deterministic bytes.
 *
 * Filled natively rather than in a loop, because some of these files are
 * hundreds of megabytes and a per-byte loop in JavaScript takes minutes.
 * Files sharing a `seed` are byte-identical, which is how the duplicate
 * fixtures are made; different seeds differ throughout, so a head-and-tail
 * probe cannot mistake them for each other.
 */
function bytes(size, seed) {
  const pattern = Buffer.from(`scuttle-demo-${seed}-`.repeat(8));
  return Buffer.alloc(size, pattern);
}

let count = 0;
function file(relative, { size = 4 * KB, days = 30, seed = 1, content = null } = {}) {
  const path = join(home, relative);
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, content ?? bytes(size, seed));
  // Age is most of what the evidence is made of — "nobody has opened this in
  // eight months" is a fact about the file, and the demo shows the real one.
  const when = Date.now() / 1000 - days * DAY;
  utimesSync(path, when, when);
  count += 1;
}

// --- A cache with as many blobs as a real one --------------------------------
// The file count is what makes a scan take time, and a scan you cannot watch
// happen is a scan the demo cannot show. These sit inside a leftover folder
// that is already one finding, so a great many files do not become a great
// many findings.
const BULK = Number(process.env.SCUTTLE_DEMO_BULK ?? 300_000);
{
  const blob = bytes(512, 'shader');
  const folders = 1_200;
  const per = Math.max(1, Math.round(BULK / folders));
  for (let f = 0; f < folders; f += 1) {
    for (let n = 0; n < per; n += 1) {
      file(`Library/Application Support/com.harborlight.tidepool/Caches/shaders/${f}/${n}.bin`, {
        days: 254,
        content: blob,
      });
    }
  }
}

// --- Software that is no longer installed ------------------------------------
// Application Support folders whose bundle identifiers match nothing on this
// machine, because these products do not exist. That is what a real leftover
// looks like, and it is why the detector finds them.
for (let n = 0; n < 14; n += 1) {
  file(`Library/Application Support/com.harborlight.tidepool/Caches/atlas-${n}.pak`, {
    size: 9 * MB,
    days: 254,
    seed: `atlas-${n}`,
  });
}
file('Library/Application Support/com.harborlight.tidepool/Logs/session.log', {
  size: 6 * MB,
  days: 254,
  seed: 'log',
});

for (let n = 0; n < 9; n += 1) {
  file(`Library/Application Support/com.pennyfarthing.almanac/Cache/volume-${n}.dat`, {
    size: 11 * MB,
    days: 402,
    seed: `vol-${n}`,
  });
}

// --- Installers that already did their job -----------------------------------
// The same application downloaded five times, which is what a browser does
// when you click the link again rather than look in Downloads first.
const marlinspike = bytes(86 * MB, 'marlinspike-3.2.1');
file('Downloads/Marlinspike-3.2.1.dmg', { days: 96, content: marlinspike });
file('Downloads/Marlinspike-3.2.1 (1).dmg', { days: 181, content: marlinspike });
file('Downloads/Marlinspike-3.2.1 (2).dmg', { days: 224, content: marlinspike });
file('Downloads/Marlinspike-3.1.0.dmg', { size: 84 * MB, days: 288, seed: 'm-310' });
file('Downloads/Marlinspike-3.0.4.dmg', { size: 81 * MB, days: 366, seed: 'm-304' });
file('Downloads/Quillfeather-2.4.1.dmg', { size: 57 * MB, days: 149, seed: 'quill' });
file('Downloads/Tessellate-1.9.4.dmg', { size: 38 * MB, days: 212, seed: 'tess' });

// --- Screenshots -------------------------------------------------------------
// The interesting finding is never one capture. It is the eleven attempts at
// the same thing on the same afternoon, none looked at again. Sized like real
// Retina captures, which are much larger than people expect.
for (let n = 1; n <= 11; n += 1) {
  file(`Desktop/Screenshot 2026-02-14 at 15.${String(20 + n).padStart(2, '0')}.41.png`, {
    size: 3 * MB + n * 64 * KB,
    days: 195,
    seed: `shot-${n}`,
  });
}
for (const [when, day] of [
  ['2025-11-02 at 09.14.07', 320],
  ['2025-12-19 at 17.02.55', 273],
  ['2026-01-08 at 11.41.30', 253],
]) {
  file(`Pictures/Screenshots/Screenshot ${when}.png`, { size: 2.6 * MB, days: day, seed: when });
}

// --- The same bytes, more than once ------------------------------------------
const brief = bytes(148 * MB, 'quarterly-brief');
file('Downloads/quarterly-brief.pdf', { days: 208, content: brief });
file('Downloads/archive/quarterly-brief.pdf', { days: 377, content: brief });
file('Desktop/quarterly-brief (final).pdf', { days: 61, content: brief });

// --- And a tidy corner, so the demo is not all findings ----------------------
// A rummage that flagged everything would be a rummage worth distrusting.
// These sit in folders Scuttle does look at, and it leaves every one alone.
file('Downloads/harbour-survey-2026.pdf', { size: 2 * MB, days: 6, seed: 'survey' });
file('Downloads/mooring-agreement.pdf', { size: 740 * KB, days: 11, seed: 'mooring' });
file('Desktop/Letter to the harbourmaster.md', { size: 14 * KB, days: 4, seed: 'letter' });
file('Desktop/Sea shanties.txt', { size: 9 * KB, days: 1, seed: 'shanties' });
file('Desktop/todo.txt', { size: 400, days: 0, seed: 'todo' });

mkdirSync(join(home, 'Library/Application Support'), { recursive: true });
console.log(`Fixture home built at ${home}\n  ${count.toLocaleString()} files, all invented`);
