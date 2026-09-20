#!/usr/bin/env node
// Builds the body of a GitHub release from CHANGELOG.md, so release notes and
// the changelog cannot disagree — there is only one place to write them.
//
// Everything after the changelog section is the same every time and is about
// getting the download open: which file to take, and what each operating
// system is going to say about an application from a developer it has never
// heard of. That belongs in the release body rather than only in the README,
// because the release page is where somebody is standing when it happens.
//
//   node scripts/release-notes.mjs <version> [checksums-file] > notes.md

import { existsSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const REPO = 'acltabontabon/scuttle';

/**
 * The body of one `## [version]` section, up to the next release heading.
 *
 * Returns null when the version has no section, which the caller treats as a
 * reason to stop rather than a reason to publish something vague.
 */
export function changelogSection(changelog, version) {
  const lines = changelog.split('\n');
  const heading = /^## \[([^\]]+)\]/;
  const start = lines.findIndex((line) => line.match(heading)?.[1] === version);
  if (start === -1) return null;

  const rest = lines.slice(start + 1);
  const end = rest.findIndex((line) => heading.test(line));
  const body = (end === -1 ? rest : rest.slice(0, end))
    // Link definitions at the foot of the file are not part of any section.
    .filter((line) => !/^\[[^\]]+\]:\s/.test(line));

  return unwrap(body).join('\n').trim();
}

/**
 * CHANGELOG.md wraps its bullets to stay readable as a file. A release body is
 * rendered like a comment, where one newline is a hard break, so without this
 * every wrapped line would arrive as its own stranded line.
 */
export function unwrap(lines) {
  const out = [];
  let held = null;
  const flush = () => {
    if (held !== null) out.push(held);
    held = null;
  };
  let fenced = false;

  for (const line of lines) {
    if (line.trimStart().startsWith('```')) {
      flush();
      fenced = !fenced;
      out.push(line);
    } else if (fenced) {
      out.push(line);
    } else if (line.trim() === '') {
      flush();
      out.push('');
    } else if (line.startsWith(' ') && held !== null) {
      held += ` ${line.trim()}`;
    } else {
      flush();
      held = line;
    }
  }
  flush();
  return out;
}

export function downloads(version) {
  const base = `https://github.com/${REPO}/releases/download/v${version}`;
  const file = (slug, ext) => `Scuttle-${version}-${slug}.${ext}`;
  return [
    '## Which one do I want?',
    '',
    '| Your computer | Download |',
    '| --- | --- |',
    `| Mac with Apple silicon (M1 and later) | [${file('macos-apple-silicon', 'dmg')}](${base}/${file('macos-apple-silicon', 'dmg')}) |`,
    `| Mac with an Intel processor | [${file('macos-intel', 'dmg')}](${base}/${file('macos-intel', 'dmg')}) |`,
    `| Windows 10 or 11, 64-bit | [${file('windows-x64-setup', 'exe')}](${base}/${file('windows-x64-setup', 'exe')}) |`,
    '',
    'Not sure which Mac you have? Apple menu → About This Mac. "Apple M1" or later',
    'means Apple silicon.',
    '',
    '### The warning you are going to get',
    '',
    'Scuttle is built without paid code-signing certificates, so neither operating',
    'system can tell you who made it. Both of them will say so once, the first time.',
    '',
    '**macOS** — open the `.dmg`, drag Scuttle to Applications, and open it from there.',
    'You will be told the developer cannot be verified. Go to **System Settings →',
    'Privacy & Security**, scroll to **Security**, and click **Open Anyway** next to',
    'Scuttle, then open it again. That button appears only after you have tried to',
    'open the app, and only for about an hour afterwards. You do this once.',
    '',
    '**Windows** — run the `-setup.exe`. Microsoft Defender SmartScreen will say it',
    'stopped an unrecognised app: click **More info**, then **Run anyway**. Scuttle',
    'installs for your user account only, so there is no administrator prompt.',
    '',
    'Please do not turn off Gatekeeper, SmartScreen or your antivirus for this — or',
    'for anything else. Neither instruction above changes a system setting.',
    '',
    '### Verifying what you downloaded',
    '',
    'Each download has a `.sha256` file beside it.',
    '',
    '```',
    'shasum -a 256 -c Scuttle-VERSION-macos-apple-silicon.dmg.sha256      # macOS',
    'Get-FileHash .\\Scuttle-VERSION-windows-x64-setup.exe -Algorithm SHA256   # Windows',
    '```',
  ]
    .join('\n')
    .replace(/VERSION/g, version);
}

export function limitations() {
  return [
    '### Worth knowing',
    '',
    '- These builds are **not signed with an Apple Developer ID and not notarized**',
    '  on macOS, and the Windows installer is **unsigned**. macOS bundles are ad-hoc',
    '  signed, which is what lets an Apple silicon build launch at all — it is not a',
    '  verified identity and it is not notarization.',
    '- **There is no automatic updater.** Come back here for the next version.',
    '- **Linux is not supported yet.** Scuttle compiles there, but it knows nothing',
    '  about where a Linux system keeps things, so there is no Linux download.',
    '- Nothing Scuttle removes goes to the Trash or the Recycle Bin. Things wait in',
    '  the drawer until you empty it, and emptying it is permanent.',
    '',
    `[Full changelog](https://github.com/${REPO}/blob/main/CHANGELOG.md) ·`,
    `[Installing and what Scuttle does](https://github.com/${REPO}#readme)`,
  ].join('\n');
}

function main() {
  const [version, checksumFile] = process.argv.slice(2);
  if (!version) {
    console.error('Usage: node scripts/release-notes.mjs <version> [checksums-file]');
    process.exit(1);
  }

  const section = changelogSection(readFileSync(join(root, 'CHANGELOG.md'), 'utf8'), version);
  if (section === null || section === '') {
    console.error(
      `CHANGELOG.md has no entry for ${version}.\n` +
        `Add a "## [${version}] - YYYY-MM-DD" section describing this release before tagging it.`,
    );
    process.exit(1);
  }

  // The demo leads the release body when there is one, pinned to this tag's
  // copy of docs/media so it cannot drift from what the release actually
  // looks like. A release cut before the recording exists simply opens with
  // the changelog rather than with a broken image.
  const parts = [];
  for (const [file, alt] of [
    [
      'demo.gif',
      'A run through Scuttle: the rummage working through 301,264 files, the findings settling into four piles, one leftover folder going into the drawer and then coming back out again',
    ],
    ['findings.png', "Scuttle's findings, drawn as four piles on a paper floor"],
  ]) {
    if (existsSync(join(root, 'docs/media', file))) {
      parts.push(`![${alt}](https://raw.githubusercontent.com/${REPO}/v${version}/docs/media/${file})`, '');
      break;
    }
  }

  parts.push(
    section,
    '',
    downloads(version),
    '',
    limitations(),
  );

  if (checksumFile) {
    const sums = readFileSync(checksumFile, 'utf8').trim();
    if (sums) parts.push('', '<details><summary>SHA-256</summary>', '', '```', sums, '```', '', '</details>');
  }

  process.stdout.write(`${parts.join('\n')}\n`);
}

if (process.argv[1] && import.meta.url === `file://${process.argv[1]}`) main();
