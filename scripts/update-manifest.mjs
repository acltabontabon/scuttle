#!/usr/bin/env node
// Builds, checks and places the manifests the in-app updater reads.
//
// The updater is told where to look in `src-tauri/tauri.conf.json`:
//
//   https://github.com/acltabontabon/scuttle/releases/download/updater-channels/<channel>.json
//
// `updater-channels` is one rolling GitHub release that holds nothing but
// `stable.json` and `alpha.json`. It exists because GitHub's own "latest"
// address ignores prereleases, and every release so far is one. Each real
// release also carries the manifest it was built with as `latest.json`, which
// never changes afterwards.
//
// Nothing here talks to the network or to GitHub. The workflows do that; this
// only decides what to say and refuses to say it when anything is missing or
// does not add up. Refusing is the point: a manifest that names a file which
// is not there, or a signature made with the wrong key, is an update that
// fails on every user's machine.
//
//   node scripts/update-manifest.mjs check-config
//   node scripts/update-manifest.mjs build   <version> <artifacts-dir> [--base-url <url>] > latest.json
//   node scripts/update-manifest.mjs check-artifact <file> <version>
//   node scripts/update-manifest.mjs verify  <latest.json> <version> [--artifacts <dir>] [--base-url <url>]
//   node scripts/update-manifest.mjs channels <version> --prerelease <true|false>
//                                             --manifest <latest.json> --existing <dir>
//                                             --out <dir> [--force]

import { createHash, createPublicKey, verify as edVerify } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { changelogSection } from './release-notes.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
export const REPO = 'acltabontabon/scuttle';

// ---- what a release must contain ------------------------------------------

/**
 * The complete matrix: one entry per operating system and architecture Scuttle
 * ships. Nothing is published unless every one of them is here, signed.
 *
 * `key` is the name the updater plugin looks for. `file` is what the release
 * workflow names the artifact — a `.app.tar.gz` on macOS, because the updater
 * replaces the application bundle and cannot use a disk image, and the NSIS
 * installer itself on Windows.
 */
export const PLATFORMS = [
  {
    key: 'darwin-aarch64',
    label: 'macOS (Apple silicon)',
    file: (version) => `Scuttle-${version}-macos-apple-silicon.app.tar.gz`,
  },
  {
    key: 'darwin-x86_64',
    label: 'macOS (Intel)',
    file: (version) => `Scuttle-${version}-macos-intel.app.tar.gz`,
  },
  {
    key: 'windows-x86_64',
    label: 'Windows (x64)',
    file: (version) => `Scuttle-${version}-windows-x64-setup.exe`,
  },
];

export const CHANNELS = ['stable', 'alpha'];

/**
 * Where a release's file lives. `baseUrl` exists only for testing an update
 * against a local server (see docs/updates.md); a real release never has one.
 */
export const assetUrl = (version, file, repo = REPO, baseUrl = null) =>
  `${baseUrl ? baseUrl.replace(/\/+$/, '') : `https://github.com/${repo}/releases/download/v${version}`}/${file}`;

// ---- versions ---------------------------------------------------------------

const SEMVER =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*)(?:\.(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*))*))?(?:\+([0-9a-zA-Z-]+(?:\.[0-9a-zA-Z-]+)*))?$/;

/** A parsed semantic version, or null. Build metadata is dropped: it has no precedence. */
export function parseVersion(text) {
  const match = typeof text === 'string' ? SEMVER.exec(text) : null;
  if (!match) return null;
  return {
    major: Number(match[1]),
    minor: Number(match[2]),
    patch: Number(match[3]),
    pre: match[4] ? match[4].split('.') : [],
  };
}

/**
 * Semver precedence: -1, 0 or 1. Numbers compare as numbers (`alpha.10` is
 * newer than `alpha.9`), text compares as text, numbers rank below text, a
 * longer set of fields ranks above a shorter one that matches so far, and a
 * version with no prerelease ranks above every prerelease of the same number.
 * Never a string comparison of the whole thing.
 */
export function compareVersions(a, b) {
  const x = typeof a === 'string' ? parseVersion(a) : a;
  const y = typeof b === 'string' ? parseVersion(b) : b;
  if (!x || !y) throw new Error(`not a version: ${!x ? a : b}`);

  for (const field of ['major', 'minor', 'patch']) {
    if (x[field] !== y[field]) return x[field] < y[field] ? -1 : 1;
  }
  if (x.pre.length === 0 && y.pre.length === 0) return 0;
  if (x.pre.length === 0) return 1;
  if (y.pre.length === 0) return -1;

  const isNumber = (part) => /^\d+$/.test(part);
  for (let i = 0; i < Math.max(x.pre.length, y.pre.length); i++) {
    const p = x.pre[i];
    const q = y.pre[i];
    if (p === undefined) return -1;
    if (q === undefined) return 1;
    if (p === q) continue;
    if (isNumber(p) && isNumber(q)) return Number(p) < Number(q) ? -1 : 1;
    if (isNumber(p)) return -1;
    if (isNumber(q)) return 1;
    return p < q ? -1 : 1;
  }
  return 0;
}

export const isPrerelease = (version) => (parseVersion(version)?.pre.length ?? 0) > 0;

// ---- minisign ---------------------------------------------------------------
//
// The updater verifies downloads with minisign signatures. The Tauri CLI writes
// them, base64-encoded, into `.sig` files, and the same base64 is what goes in
// the manifest. Checking them here, natively, means the release workflow needs
// no extra tool and can prove — before anything is published — that what it is
// about to offer will pass the check on somebody's machine.

const ED25519_SPKI_PREFIX = Buffer.from('302a300506032b6570032100', 'hex');
const b64 = (text) => Buffer.from(String(text).trim(), 'base64');

function lines(buffer) {
  return buffer
    .toString('utf8')
    .split(/\r?\n/)
    .filter((line) => line !== '');
}

/** The key id and key of a Tauri public key (the base64 of a minisign `.pub` file). */
export function decodePublicKey(pubkey) {
  const parts = lines(b64(pubkey));
  const raw = parts.length >= 2 ? b64(parts[1]) : Buffer.alloc(0);
  if (raw.length !== 42 || raw.subarray(0, 2).toString('latin1') !== 'Ed') {
    throw new Error('not a minisign public key');
  }
  return { keyId: raw.subarray(2, 10), key: raw.subarray(10) };
}

/** The parts of a Tauri signature (the base64 of a minisign `.sig` file). */
export function decodeSignature(signature) {
  const parts = lines(b64(signature));
  if (parts.length < 4) throw new Error('not a minisign signature');
  const raw = b64(parts[1]);
  const algorithm = raw.subarray(0, 2).toString('latin1');
  if (raw.length !== 74 || (algorithm !== 'Ed' && algorithm !== 'ED')) {
    throw new Error('not a minisign signature');
  }
  const prefix = 'trusted comment: ';
  if (!parts[2].startsWith(prefix)) throw new Error('signature has no trusted comment');
  const global = b64(parts[3]);
  if (global.length !== 64) throw new Error('signature has no global signature');
  return {
    algorithm,
    keyId: raw.subarray(2, 10),
    signature: raw.subarray(10),
    trustedComment: parts[2].slice(prefix.length),
    globalSignature: global,
  };
}

/** The version a signature says it was made for, or null. */
export function signedVersion(trustedComment) {
  for (const field of trustedComment.split('\t')) {
    if (field.startsWith('version:')) return field.slice('version:'.length);
  }
  return null;
}

/**
 * Whether `signature` is a valid signature of `data` by the key in `pubkey`,
 * and if not, why. Checks both signatures a minisign file carries: the one over
 * the file, and the global one that makes the trusted comment — where the
 * signed version lives — trustworthy too.
 */
export function verifySignature(data, signature, pubkey) {
  let pub;
  let sig;
  try {
    pub = decodePublicKey(pubkey);
    sig = decodeSignature(signature);
  } catch (error) {
    return { ok: false, reason: error.message };
  }
  if (!pub.keyId.equals(sig.keyId)) {
    return { ok: false, reason: 'signed with a different key than the one in the application' };
  }

  const key = createPublicKey({
    key: Buffer.concat([ED25519_SPKI_PREFIX, pub.key]),
    format: 'der',
    type: 'spki',
  });
  const message = sig.algorithm === 'ED' ? createHash('blake2b512').update(data).digest() : data;
  if (!edVerify(null, message, key, sig.signature)) {
    return { ok: false, reason: 'the signature does not match the file' };
  }
  const global = Buffer.concat([sig.signature, Buffer.from(sig.trustedComment, 'utf8')]);
  if (!edVerify(null, global, key, sig.globalSignature)) {
    return { ok: false, reason: 'the signature’s comment has been altered' };
  }
  return { ok: true, trustedComment: sig.trustedComment, version: signedVersion(sig.trustedComment) };
}

// ---- building a manifest ----------------------------------------------------

/**
 * The manifest for one release. Throws when anything the release needs is
 * missing, rather than producing something that looks complete.
 */
export function buildManifest({ version, notes, pubDate, signatures, repo = REPO, baseUrl = null }) {
  if (!parseVersion(version)) throw new Error(`"${version}" is not a semantic version`);
  const platforms = {};
  for (const platform of PLATFORMS) {
    const signature = signatures[platform.key];
    if (typeof signature !== 'string' || signature.trim() === '') {
      throw new Error(`no signature for ${platform.label} (${platform.file(version)}.sig)`);
    }
    platforms[platform.key] = {
      signature: signature.trim(),
      url: assetUrl(version, platform.file(version), repo, baseUrl),
    };
  }
  return { version, notes: notes ?? '', pub_date: pubDate, platforms };
}

/** Everything wrong with a manifest, as sentences. Empty means it can be published. */
export function manifestProblems(
  manifest,
  { version, pubkey, repo = REPO, artifacts = null, requireSignedVersion = true, baseUrl = null },
) {
  const problems = [];
  const add = (text) => problems.push(text);

  if (!manifest || typeof manifest !== 'object') return ['the manifest is not an object'];
  if (manifest.version !== version) {
    add(`version is "${manifest.version}", expected "${version}"`);
  }
  if (!parseVersion(manifest.version)) add(`"${manifest.version}" is not a semantic version`);
  if (typeof manifest.notes !== 'string') add('notes is not a string');
  if (typeof manifest.pub_date !== 'string' || Number.isNaN(Date.parse(manifest.pub_date))) {
    add('pub_date is not an RFC 3339 date');
  }

  const platforms = manifest.platforms;
  if (!platforms || typeof platforms !== 'object') {
    add('platforms is missing');
    return problems;
  }

  const expected = new Set(PLATFORMS.map((p) => p.key));
  for (const key of Object.keys(platforms)) {
    if (!expected.has(key)) add(`unexpected platform "${key}"`);
  }

  let publicKey = null;
  try {
    publicKey = decodePublicKey(pubkey);
  } catch {
    add('the public key is not a valid minisign key');
  }

  for (const platform of PLATFORMS) {
    const entry = platforms[platform.key];
    const name = `${platform.label} (${platform.key})`;
    if (!entry) {
      add(`${name} is missing`);
      continue;
    }

    const file = platform.file(manifest.version ?? version);
    const wantUrl = assetUrl(version, file, repo, baseUrl);
    // https is required unless this is a local test with an explicit base.
    if (typeof entry.url !== 'string' || (!baseUrl && !entry.url.startsWith('https://'))) {
      add(`${name}: the URL is not https`);
    } else if (entry.url !== wantUrl) {
      add(`${name}: the URL is ${entry.url}, expected ${wantUrl}`);
    }

    if (typeof entry.signature !== 'string' || entry.signature.trim() === '') {
      add(`${name}: there is no signature`);
      continue;
    }
    let decoded;
    try {
      decoded = decodeSignature(entry.signature);
    } catch (error) {
      add(`${name}: ${error.message}`);
      continue;
    }
    if (publicKey && !publicKey.keyId.equals(decoded.keyId)) {
      add(`${name}: signed with a different key than the one in the application`);
      continue;
    }

    // The application is built to refuse a signature that does not say which
    // version it was made for. If the release were signed without that, every
    // update would fail on every machine, so it is checked here instead.
    if (requireSignedVersion) {
      const signed = signedVersion(decoded.trustedComment);
      if (signed === null) {
        add(`${name}: the signature does not record a version, which the application requires`);
      } else if (signed !== version) {
        add(`${name}: the signature was made for version ${signed}`);
      }
    }

    if (artifacts) {
      const path = join(artifacts, file);
      if (!existsSync(path)) {
        add(`${name}: ${file} is not in ${artifacts}`);
      } else {
        const outcome = verifySignature(readFileSync(path), entry.signature, pubkey);
        if (!outcome.ok) add(`${name}: ${file} does not verify — ${outcome.reason}`);
      }
    }
  }

  return problems;
}

// ---- channels ---------------------------------------------------------------

/**
 * Which channel pointers a release should be written to.
 *
 * * A stable release goes to both: stable users get it, and alpha users move
 *   on from the alphas that led up to it.
 * * A prerelease goes to alpha only. Stable never sees it.
 * * A pointer is only replaced by something newer. Publishing an old patch
 *   after a newer release does not roll anybody back; that takes `force`, for
 *   the case where a bad release is being deliberately withdrawn.
 *
 * `existing` maps a channel to the manifest currently published there, if any.
 */
export function pointersToWrite({ version, prerelease, existing = {}, force = false }) {
  const wanted = prerelease ? ['alpha'] : ['stable', 'alpha'];
  return wanted.filter((channel) => {
    const current = existing[channel]?.version;
    if (force || !current || !parseVersion(current)) return true;
    return compareVersions(version, current) > 0;
  });
}

// ---- reading the application's own configuration -----------------------------

export function readUpdaterConfig(path = join(root, 'src-tauri/tauri.conf.json')) {
  const config = JSON.parse(readFileSync(path, 'utf8'));
  return config.plugins?.updater ?? null;
}

/** Problems that mean this checkout cannot ship a working updater. */
export function configProblems(updater) {
  const problems = [];
  if (!updater) return ['plugins.updater is missing from tauri.conf.json'];

  try {
    const { keyId } = decodePublicKey(updater.pubkey ?? '');
    if (keyId.every((byte) => byte === 0)) problems.push('the public key has an empty key id');
  } catch {
    problems.push(
      'plugins.updater.pubkey is not a real public key yet — generate one with `npm run tauri signer generate` (see docs/updates.md)',
    );
  }

  const endpoints = updater.endpoints ?? [];
  if (endpoints.length === 0) problems.push('there are no update endpoints');
  for (const endpoint of endpoints) {
    if (!String(endpoint).startsWith('https://')) problems.push(`the endpoint ${endpoint} is not https`);
  }
  if (!endpoints.some((endpoint) => String(endpoint).includes('{{channel}}'))) {
    problems.push('no endpoint uses {{channel}}, so every install would read the same manifest');
  }
  if (updater.requireSignedVersion !== true) {
    problems.push('requireSignedVersion is not on, which leaves the downgrade check open');
  }
  if (updater.dangerousInsecureTransportProtocol) {
    problems.push('dangerousInsecureTransportProtocol must never be on in the shipped configuration');
  }
  return problems;
}

// ---- command line -------------------------------------------------------------

function flags(args) {
  const positional = [];
  const named = {};
  for (let i = 0; i < args.length; i++) {
    if (args[i].startsWith('--')) {
      const name = args[i].slice(2);
      const next = args[i + 1];
      if (next === undefined || next.startsWith('--')) named[name] = true;
      else {
        named[name] = next;
        i++;
      }
    } else positional.push(args[i]);
  }
  return { positional, named };
}

function fail(lines) {
  for (const line of [].concat(lines)) console.error(`::error::${line}`);
  process.exit(1);
}

function main() {
  const [command, ...rest] = process.argv.slice(2);
  const { positional, named } = flags(rest);

  switch (command) {
    case 'check-config': {
      const problems = configProblems(readUpdaterConfig());
      if (problems.length) fail(problems);
      console.error('The updater configuration is complete.');
      return;
    }

    case 'build': {
      const [version, dir] = positional;
      if (!version || !dir) fail('Usage: build <version> <artifacts-dir>');
      const signatures = {};
      for (const platform of PLATFORMS) {
        const path = join(dir, `${platform.file(version)}.sig`);
        if (existsSync(path)) signatures[platform.key] = readFileSync(path, 'utf8');
      }
      const changelog = readFileSync(join(root, 'CHANGELOG.md'), 'utf8');
      try {
        const manifest = buildManifest({
          version,
          notes: changelogSection(changelog, version) ?? '',
          pubDate: new Date().toISOString(),
          signatures,
          baseUrl: typeof named['base-url'] === 'string' ? named['base-url'] : null,
        });
        process.stdout.write(`${JSON.stringify(manifest, null, 2)}\n`);
      } catch (error) {
        fail(error.message);
      }
      return;
    }

    case 'check-artifact': {
      // One built artifact against its own `.sig`, right after it was built:
      // the earliest point a wrong key, a missing signature or an unbound
      // version can be caught, on the machine that made it.
      const [file, version] = positional;
      if (!file || !version) fail('Usage: check-artifact <file> <version>');
      const updater = readUpdaterConfig();
      if (!existsSync(file)) fail(`${file} does not exist`);
      if (!existsSync(`${file}.sig`)) fail(`${file}.sig does not exist — the build did not sign it`);
      const outcome = verifySignature(readFileSync(file), readFileSync(`${file}.sig`, 'utf8'), updater?.pubkey ?? '');
      if (!outcome.ok) fail(`${file} does not verify: ${outcome.reason}`);
      if (updater?.requireSignedVersion === true && outcome.version !== version) {
        fail(`${file} was signed for version ${outcome.version ?? '(none)'}, expected ${version}`);
      }
      console.error(`${file} verifies against the key in the application, signed for ${outcome.version}.`);
      return;
    }

    case 'verify': {
      const [file, version] = positional;
      if (!file || !version) fail('Usage: verify <manifest.json> <version> [--artifacts <dir>]');
      const updater = readUpdaterConfig();
      let manifest;
      try {
        manifest = JSON.parse(readFileSync(file, 'utf8'));
      } catch (error) {
        fail(`${file} is not valid JSON: ${error.message}`);
      }
      const problems = manifestProblems(manifest, {
        version,
        pubkey: updater?.pubkey ?? '',
        artifacts: typeof named.artifacts === 'string' ? named.artifacts : null,
        requireSignedVersion: updater?.requireSignedVersion === true,
        baseUrl: typeof named['base-url'] === 'string' ? named['base-url'] : null,
      });
      if (problems.length) fail(problems);
      console.error(
        `${file} is complete: ${PLATFORMS.length} platforms, every signature checked${named.artifacts ? ' against its file' : ''}.`,
      );
      return;
    }

    case 'channels': {
      const [version] = positional;
      if (!version || typeof named.out !== 'string' || typeof named.manifest !== 'string') {
        fail(
          'Usage: channels <version> --prerelease <true|false> --manifest <latest.json> --existing <dir> --out <dir> [--force]',
        );
      }
      const existing = {};
      for (const channel of CHANNELS) {
        const path = typeof named.existing === 'string' ? join(named.existing, `${channel}.json`) : null;
        if (path && existsSync(path)) {
          try {
            existing[channel] = JSON.parse(readFileSync(path, 'utf8'));
          } catch {
            // A pointer nobody can read is one worth replacing.
          }
        }
      }
      const write = pointersToWrite({
        version,
        prerelease: named.prerelease === 'true',
        existing,
        force: Boolean(named.force),
      });
      mkdirSync(named.out, { recursive: true });
      const source = readFileSync(named.manifest, 'utf8');
      for (const channel of write) writeFileSync(join(named.out, `${channel}.json`), source);
      // Which files to upload, one per line, for the workflow to read.
      console.log(write.map((channel) => `${channel}.json`).join('\n'));
      return;
    }

    default:
      fail('Usage: update-manifest.mjs <check-config|build|verify|channels> …');
  }
}

if (process.argv[1] && import.meta.url === `file://${process.argv[1]}`) main();
