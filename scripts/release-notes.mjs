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
    '## Bring Scuttle home',
    '',
    'Choose your download. No account. No cloud scanning. Your files stay on your computer.',
    '',
    '| Your computer | Get Scuttle |',
    '| --- | --- |',
    `| **Mac · Apple silicon** (M1 and later, macOS 11+) | **[Download for Apple silicon →](${base}/${file('macos-apple-silicon', 'dmg')})** |`,
    `| **Mac · Intel** (macOS 10.15+) | **[Download for Intel →](${base}/${file('macos-intel', 'dmg')})** |`,
    `| **Windows** 10 or 11, 64-bit | **[Download for Windows →](${base}/${file('windows-x64-setup', 'exe')})** |`,
    '',
    'Not sure which Mac you have? Apple menu → About This Mac. "Apple M1" or later means Apple silicon.',
    '',
    '**First launch:** macOS builds are ad-hoc signed, without an Apple Developer ID or notarization; the Windows installer is unsigned. Expect an operating-system warning. Installation steps are below.',
    '',
    '<details>',
    '<summary><strong>Installation & first-launch help</strong></summary>',
    '',
    'Scuttle is built without paid code-signing certificates, so neither operating system can verify the publisher.',
    '',
    '**macOS** — open the `.dmg`, drag Scuttle to Applications, and open it from there. You will be told the developer cannot be verified. Go to **System Settings → Privacy & Security**, scroll to **Security**, and click **Open Anyway** next to Scuttle, then open it again. That button appears only after you have tried to open the app, and only for about an hour afterwards. You do this once.',
    '',
    'If macOS says **damaged**, check the checksum and download again; that is not the expected first-launch prompt.',
    '',
    '**Windows** — run the `-setup.exe`. Microsoft Defender SmartScreen will say it stopped an unrecognised app: click **More info**, then **Run anyway**. Scuttle installs for your user account only, so there is no administrator prompt. The warning may appear again for a new version.',
    '',
    'Keep Gatekeeper, SmartScreen and your antivirus enabled. These steps allow Scuttle to open without disabling those protections.',
    '',
    '</details>',
    '',
    '<details>',
    '<summary><strong>Verify your download</strong></summary>',
    '',
    'Each download has a `.sha256` file beside it.',
    '',
    '```',
    'shasum -a 256 -c Scuttle-VERSION-macos-apple-silicon.dmg.sha256      # macOS',
    'Get-FileHash .\\Scuttle-VERSION-windows-x64-setup.exe -Algorithm SHA256   # Windows',
    '```',
    '',
    'On Windows, compare the hash with the matching entry in `SHA256SUMS.txt`.',
    '',
    '</details>',
  ]
    .join('\n')
    .replace(/VERSION/g, version);
}

export function limitations(version) {
  return unwrap([
    '## You stay in charge',
    '',
    '| Find it | Understand it | Decide what stays |',
    '| --- | --- | --- |',
    '| Rummage through leftovers, screenshots, installers, caches and duplicates. | See the evidence, confidence and risk behind each finding. | Review moves, keep items you want, and restore from the drawer. |',
    '',
    '**Organization gives files a home in ordinary folders.** It does not free storage, and organized files never expire. Find Undo in Organization history; it checks each file and leaves changed files or conflicting original paths alone.',
    '',
    '**The drawer keeps a moved copy on disk.** Moving something there does not free space. Only permanently removing it does. Caches and re-downloadable installers may expire after your chosen retention period; other items stay until you remove them.',
    '',
    '<details>',
    '<summary><strong>Updates & platform details</strong></summary>',
    '',
    '- These builds are **not signed with an Apple Developer ID and not notarized**',
    '  on macOS, and the Windows installer is **unsigned**. macOS bundles are ad-hoc',
    '  signed, which is what lets an Apple silicon build launch at all — it is not a',
    '  verified identity and it is not notarization.',
    '- **Scuttle can update itself**, if you let it: it checks about once a day and',
    '  only ever asks before downloading or restarting. Copies older than the first',
    '  release with updates cannot; they need this download installed by hand once.',
    '- **Linux is not supported yet.** Scuttle compiles there, but it knows nothing',
    '  about where a Linux system keeps things, so there is no Linux download.',
    '- Nothing goes to the Trash or the Recycle Bin. Emptying the drawer is permanent.',
    '- Whole folders on another drive cannot be moved into the drawer.',
    '',
    '</details>',
    '',
    '---',
    '',
    `**[Explore Scuttle](https://acltabontabon.com/scuttle/) · [Read the guide](https://github.com/${REPO}/blob/v${version}/README.md) · [Full changelog](https://github.com/${REPO}/blob/v${version}/CHANGELOG.md) · [Report an issue](https://github.com/${REPO}/issues)**`,
  ]).join('\n');
}

export function releaseBody(version, section, { media = null, checksums = '' } = {}) {
  const parts = [
    '<div align="center">',
    '',
    `<img src="https://raw.githubusercontent.com/${REPO}/v${version}/src-tauri/icons/128x128.png" width="80" alt="Scuttle">`,
    '',
    '# Find what’s out of place.<br>Decide where it belongs.',
    '',
    '**Make room. Gather scattered files. Keep the final say.**',
    '',
    `Scuttle ${version} · macOS & Windows · Free & open source`,
    '',
    '</div>',
    '',
  ];
  if (media) parts.push(`![Scuttle: review what turned up and choose where it belongs](https://raw.githubusercontent.com/${REPO}/v${version}/docs/media/${media})`, '');
  const detailStart = section.includes('### Highlights') ? section.search(/^### (Fixed|Changed|Added)$/m) : -1;
  const summary = detailStart < 0 ? section : section.slice(0, detailStart).trim();
  parts.push('## What’s new', '', summary, '', downloads(version));
  if (detailStart >= 0) parts.push('', '<details>', '<summary><strong>All fixes & improvements</strong></summary>', '', section.slice(detailStart).trim(), '', '</details>');
  parts.push('', limitations(version));
  if (checksums.trim()) parts.push('', '<details>', '<summary><strong>SHA-256 checksums</strong></summary>', '', '```text', checksums.trim(), '```', '', '</details>');
  return `${parts.join('\n')}\n`;
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
  const media = ['ui-findings.jpg', 'demo.gif', 'findings.png'].find((file) => existsSync(join(root, 'docs/media', file)));
  const checksums = checksumFile ? readFileSync(checksumFile, 'utf8') : '';
  process.stdout.write(releaseBody(version, section, { media, checksums }));
}

if (process.argv[1] && import.meta.url === `file://${process.argv[1]}`) main();
