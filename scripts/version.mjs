#!/usr/bin/env node
// Scuttle's version lives in five places and has to agree in all of them.
//
//   package.json          what npm thinks the project is
//   package-lock.json     twice: the root, and the "" entry for the root package
//   src-tauri/Cargo.toml  the crate version
//   src-tauri/Cargo.lock  the crate's own entry in its own lockfile
//   tauri.conf.json       what goes into the .app's Info.plist and the installer
//
// Only the Cargo one reaches the About screen (the `about` command reads
// CARGO_PKG_VERSION), which is exactly why the others drift without anyone
// noticing. One command sets all five; another refuses a release whose tag
// disagrees with any of them.
//
//   npm run version:set -- 0.2.0        set everything to 0.2.0
//   npm run version:check                are they all the same?
//   npm run version:check -- v0.2.0      ...and do they match this tag?

import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

// Semantic versions, optionally with a prerelease and build suffix. Deliberately
// stricter than semver allows for the leading numbers: no leading zeroes, so
// `0.10.0` and `0.010.0` can never both exist as tags.
const SEMVER =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*)(?:\.(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*))*))?(?:\+([0-9a-zA-Z-]+(?:\.[0-9a-zA-Z-]+)*))?$/;

const read = (rel) => readFileSync(join(root, rel), 'utf8');
const write = (rel, text) => writeFileSync(join(root, rel), text);

/** Where each version lives, how to read it, and how to rewrite it in place. */
const sites = [
  {
    file: 'package.json',
    label: 'package.json',
    read: (text) => JSON.parse(text).version,
    // Edited as text rather than re-serialised, so npm's own formatting and
    // key order survive untouched.
    write: (text, next) => replaceOnce(text, JSON_VERSION, next, 'package.json version'),
  },
  {
    file: 'package-lock.json',
    label: 'package-lock.json',
    read: (text) => JSON.parse(text).version,
    write: (text, next) => {
      const lock = JSON.parse(text);
      lock.version = next;
      if (lock.packages?.['']) lock.packages[''].version = next;
      return `${JSON.stringify(lock, null, 2)}\n`;
    },
  },
  {
    file: 'src-tauri/Cargo.toml',
    label: 'Cargo.toml',
    read: (text) => matchOnce(text, CARGO_VERSION, 'Cargo.toml version'),
    write: (text, next) => replaceOnce(text, CARGO_VERSION, next, 'Cargo.toml version'),
  },
  {
    file: 'src-tauri/Cargo.lock',
    label: 'Cargo.lock',
    // Only Scuttle's own entry, never a dependency that happens to share a
    // version number.
    read: (text) => matchOnce(text, LOCK_ENTRY, 'scuttle entry in Cargo.lock'),
    write: (text, next) => replaceOnce(text, LOCK_ENTRY, next, 'scuttle entry in Cargo.lock'),
  },
  {
    file: 'src-tauri/tauri.conf.json',
    label: 'tauri.conf.json',
    read: (text) => JSON.parse(text).version,
    write: (text, next) => replaceOnce(text, JSON_VERSION, next, 'tauri.conf.json version'),
  },
];

// Every pattern is (what comes before)(the version)(what comes after), so
// reading and rewriting can share one shape and neither has to count groups.
//
// `\r?\n` rather than `\n`, because git hands these files over with CRLF on
// Windows and a literal `\n` then matches nothing — which is a confusing way
// to fail on one platform only. The single-line patterns are safe already:
// `$` in multiline mode treats a carriage return as a line terminator.
const JSON_VERSION = /^(\s*"version":\s*")([^"]*)(",)$/m;
const CARGO_VERSION = /^(version = ")([^"]*)(")$/m;
const LOCK_ENTRY = /(\[\[package\]\]\r?\nname = "scuttle"\r?\nversion = ")([^"]*)(")/;

function matchOnce(text, pattern, what) {
  const found = text.match(pattern);
  if (!found) throw new Error(`Could not find the ${what}.`);
  return found[2];
}

function replaceOnce(text, pattern, next, what) {
  if (!pattern.test(text)) throw new Error(`Could not find the ${what}.`);
  return text.replace(pattern, (_match, before, _version, after) => `${before}${next}${after}`);
}

function current() {
  return sites.map((site) => ({ ...site, version: site.read(read(site.file)) }));
}

function set(next) {
  if (!SEMVER.test(next)) {
    fail(
      `"${next}" is not a semantic version.\n` +
        'Expected something like 0.2.0, or 0.2.0-rc.1 for a prerelease.',
    );
  }
  for (const site of sites) write(site.file, site.write(read(site.file), next));

  console.log(`Scuttle is now ${next}:`);
  for (const site of sites) console.log(`  ${site.label}`);
  console.log(
    '\nNext: describe the release under a dated heading in CHANGELOG.md,' +
      '\ncommit, then tag it v' +
      next +
      '. docs/releasing.md has the rest.',
  );
}

function check(tag) {
  const found = current();
  const versions = [...new Set(found.map((site) => site.version))];

  if (versions.length > 1) {
    fail(
      'The version is not the same everywhere:\n' +
        found.map((site) => `  ${site.version.padEnd(16)} ${site.label}`).join('\n') +
        '\n\nRun: npm run version:set -- <version>',
    );
  }

  const [version] = versions;
  if (!SEMVER.test(version)) fail(`"${version}" is not a semantic version.`);

  if (tag) {
    // Tags are vX.Y.Z. A tag that says one thing while the binary reports
    // another is the one release mistake nobody can fix after the fact.
    if (!tag.startsWith('v')) fail(`Release tags start with "v". Got "${tag}".`);
    const tagged = tag.slice(1);
    if (tagged !== version) {
      fail(
        `Tag ${tag} does not match the application version ${version}.\n` +
          `Either tag v${version}, or run: npm run version:set -- ${tagged}`,
      );
    }
    console.log(`${tag} matches the application version everywhere.`);
    return;
  }

  console.log(`Scuttle is ${version} everywhere.`);
}

function fail(message) {
  console.error(message);
  process.exit(1);
}

const [command, argument] = process.argv.slice(2);
try {
  if (command === 'set') {
    if (!argument) fail('Usage: npm run version:set -- <version>');
    set(argument);
  } else if (command === 'check') {
    check(argument);
  } else {
    fail('Usage: node scripts/version.mjs set <version> | check [tag]');
  }
} catch (error) {
  fail(error.message);
}
