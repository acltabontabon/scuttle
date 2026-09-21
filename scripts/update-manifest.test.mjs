// @vitest-environment node
//
// The fixtures below are real: two throwaway keypairs made with
// `tauri signer generate`, and a signature made with the first over the bytes
// "scuttle test artifact", bound to version 0.1.0-alpha.3 with `--app-version`.
// Only public keys and a signature are kept; the private halves were discarded
// with the temporary directory they were made in. They protect nothing.

import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import {
  assetUrl,
  buildManifest,
  compareVersions,
  configProblems,
  decodePublicKey,
  isPrerelease,
  manifestProblems,
  PLATFORMS,
  pointersToWrite,
  verifySignature,
} from './update-manifest.mjs';

const KEY_A =
  'dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDVFRjFBODBBOEE1RTQ2NTgKUldSWVJsNktDcWp4WHRwZW5HMS9VRDBGdjRHWDdLcXRNanZtQVV5VW16UmQyaTc2ajY4UVNHem4K';
const KEY_B =
  'dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDVBQTUyODMwNEZBOTJENzkKUldSNUxhbFBNQ2lsV2hsSkF3dHR3NW80WmJKTkNkdjJsaG1iM0lSVGlWQklKRTVRTWFaVFRPWm0K';
const SIGNATURE =
  'dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVSWVJsNktDcWp4WHRxdnFqNGlJVjZNQW1NaUZoaEw0cjh3VitPZHFSZDhMenc2Z2JvRFZJSE5mZG9aL3NYSDV3NnZ2eTFtRlhxd0gvY1dPWU1xcURYRkhMcVdHdVVubUFNPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzg5OTg4NTA5CWZpbGU6YXJ0LmJpbgl2ZXJzaW9uOjAuMS4wLWFscGhhLjMKR0RFZ29WWWhzMzFER3REUE5UN3ZkUWxmaDY1cGtvejk5UmpqL2NjbWJOaytlSDM4UmdsOTNLZmIwWlpET3NKVzRHMktvSVRnL294UVFaR1k3VzV4Q1E9PQo=';
const ARTIFACT = Buffer.from('scuttle test artifact');
const VERSION = '0.1.0-alpha.3';

const allSigned = () => Object.fromEntries(PLATFORMS.map((p) => [p.key, SIGNATURE]));
const manifest = (over = {}) =>
  buildManifest({
    version: VERSION,
    notes: '### Added\n- A thing.',
    pubDate: '2026-09-21T00:00:00Z',
    signatures: allSigned(),
    ...over,
  });
const problems = (m, over = {}) =>
  manifestProblems(m, { version: VERSION, pubkey: KEY_A, ...over });

// ---- version comparison -------------------------------------------------------

describe('compareVersions', () => {
  const newer = (a, b) => compareVersions(a, b) === 1 && compareVersions(b, a) === -1;

  it('compares numbers as numbers, not as text', () => {
    expect(newer('0.1.0-alpha.10', '0.1.0-alpha.9')).toBe(true);
    expect(newer('0.10.0', '0.9.0')).toBe(true);
  });

  it('ranks a release above every prerelease of it', () => {
    expect(newer('0.1.0', '0.1.0-alpha.99')).toBe(true);
    expect(newer('0.1.0', '0.1.0-rc.1')).toBe(true);
  });

  it('follows the semver precedence rules for prerelease fields', () => {
    // The order from the semver specification, item 11.
    const order = [
      '1.0.0-alpha',
      '1.0.0-alpha.1',
      '1.0.0-alpha.beta',
      '1.0.0-beta',
      '1.0.0-beta.2',
      '1.0.0-beta.11',
      '1.0.0-rc.1',
      '1.0.0',
    ];
    for (let i = 1; i < order.length; i++) {
      expect(compareVersions(order[i - 1], order[i]), `${order[i - 1]} < ${order[i]}`).toBe(-1);
    }
  });

  it('ignores build metadata', () => {
    expect(compareVersions('1.0.0+a', '1.0.0+b')).toBe(0);
  });

  it('refuses something that is not a version', () => {
    expect(() => compareVersions('latest', '1.0.0')).toThrow();
  });

  it('knows a prerelease when it sees one', () => {
    expect(isPrerelease('0.1.0-alpha.2')).toBe(true);
    expect(isPrerelease('0.1.0')).toBe(false);
  });
});

// ---- which channels a release reaches --------------------------------------------

describe('pointersToWrite', () => {
  it('sends a prerelease to alpha only', () => {
    expect(pointersToWrite({ version: '0.1.0-alpha.3', prerelease: true })).toEqual(['alpha']);
  });

  it('sends a stable release to both, so alphas move on to it', () => {
    expect(pointersToWrite({ version: '0.1.0', prerelease: false })).toEqual(['stable', 'alpha']);
  });

  it('never replaces a pointer with something that is not newer', () => {
    const existing = { alpha: { version: '0.2.0-alpha.1' }, stable: { version: '0.1.0' } };
    expect(pointersToWrite({ version: '0.1.1-alpha.4', prerelease: true, existing })).toEqual([]);
    expect(pointersToWrite({ version: '0.1.0', prerelease: false, existing })).toEqual([]);
    expect(pointersToWrite({ version: '0.1.1', prerelease: false, existing })).toEqual(['stable']);
  });

  it('replaces a pointer that nobody can read', () => {
    const existing = { alpha: { version: 'not a version' } };
    expect(pointersToWrite({ version: '0.1.0-alpha.3', prerelease: true, existing })).toEqual(['alpha']);
  });

  it('can be forced, for withdrawing a bad release on purpose', () => {
    const existing = { alpha: { version: '0.1.0-alpha.9' } };
    expect(
      pointersToWrite({ version: '0.1.0-alpha.3', prerelease: true, existing, force: true }),
    ).toEqual(['alpha']);
  });
});

// ---- building ---------------------------------------------------------------------------

describe('buildManifest', () => {
  it('names every platform, with the right file at the right address', () => {
    const m = manifest();
    expect(Object.keys(m.platforms).sort()).toEqual(
      ['darwin-aarch64', 'darwin-x86_64', 'windows-x86_64'].sort(),
    );
    expect(m.platforms['darwin-aarch64'].url).toBe(
      assetUrl(VERSION, `Scuttle-${VERSION}-macos-apple-silicon.app.tar.gz`),
    );
    expect(m.platforms['windows-x86_64'].url).toBe(
      `https://github.com/acltabontabon/scuttle/releases/download/v${VERSION}/Scuttle-${VERSION}-windows-x64-setup.exe`,
    );
    expect(m.version).toBe(VERSION);
  });

  it('refuses when a platform has no signature', () => {
    const signatures = allSigned();
    delete signatures['windows-x86_64'];
    expect(() => manifest({ signatures })).toThrow(/Windows/);
    signatures['windows-x86_64'] = '   ';
    expect(() => manifest({ signatures })).toThrow(/Windows/);
  });

  it('refuses something that is not a version', () => {
    expect(() => manifest({ version: 'v1' })).toThrow();
  });
});

// ---- signatures ---------------------------------------------------------------------------

describe('verifySignature', () => {
  it('accepts the file it was made for', () => {
    const result = verifySignature(ARTIFACT, SIGNATURE, KEY_A);
    expect(result.ok).toBe(true);
    expect(result.version).toBe(VERSION);
  });

  it('rejects a file that was changed by a single byte', () => {
    const changed = Buffer.from(ARTIFACT);
    changed[0] ^= 1;
    expect(verifySignature(changed, SIGNATURE, KEY_A)).toMatchObject({
      ok: false,
      reason: expect.stringContaining('does not match'),
    });
  });

  it('rejects a signature made with a different key', () => {
    expect(verifySignature(ARTIFACT, SIGNATURE, KEY_B)).toMatchObject({
      ok: false,
      reason: expect.stringContaining('different key'),
    });
  });

  it('rejects a signature whose comment was altered', () => {
    const text = Buffer.from(SIGNATURE, 'base64').toString('utf8');
    const forged = Buffer.from(text.replace('version:0.1.0-alpha.3', 'version:9.9.9')).toString('base64');
    expect(verifySignature(ARTIFACT, forged, KEY_A)).toMatchObject({
      ok: false,
      reason: expect.stringContaining('comment'),
    });
  });

  it('rejects garbage without throwing', () => {
    expect(verifySignature(ARTIFACT, 'not a signature', KEY_A).ok).toBe(false);
    expect(verifySignature(ARTIFACT, SIGNATURE, 'not a key').ok).toBe(false);
    expect(verifySignature(ARTIFACT, '', KEY_A).ok).toBe(false);
  });
});

// ---- verifying a manifest --------------------------------------------------------------------

describe('manifestProblems', () => {
  it('accepts a complete, correct manifest', () => {
    expect(problems(manifest())).toEqual([]);
  });

  it('notices a missing platform', () => {
    const m = manifest();
    delete m.platforms['darwin-x86_64'];
    expect(problems(m)).toEqual([expect.stringContaining('macOS (Intel)')]);
  });

  it('notices a platform nobody asked for', () => {
    const m = manifest();
    m.platforms['linux-x86_64'] = m.platforms['windows-x86_64'];
    expect(problems(m)).toEqual([expect.stringContaining('unexpected platform "linux-x86_64"')]);
  });

  it('notices a manifest for the wrong version', () => {
    expect(problems(manifest(), { version: '0.1.0-alpha.4' }).join('\n')).toMatch(
      /version is "0.1.0-alpha.3", expected "0.1.0-alpha.4"/,
    );
  });

  it('notices an address that is not https', () => {
    const m = manifest();
    m.platforms['windows-x86_64'].url = m.platforms['windows-x86_64'].url.replace('https', 'http');
    expect(problems(m).join('\n')).toMatch(/not https/);
  });

  it('notices an address that points somewhere else', () => {
    const m = manifest();
    m.platforms['darwin-aarch64'].url = 'https://example.test/Scuttle.app.tar.gz';
    expect(problems(m).join('\n')).toMatch(/expected https:\/\/github.com\/acltabontabon\/scuttle/);
  });

  it('notices a missing or malformed signature', () => {
    const m = manifest();
    m.platforms['darwin-aarch64'].signature = '';
    m.platforms['windows-x86_64'].signature = 'AAAA';
    const found = problems(m);
    expect(found).toHaveLength(2);
    expect(found.join('\n')).toMatch(/no signature/);
  });

  it('notices a signature made with the wrong key', () => {
    expect(problems(manifest(), { pubkey: KEY_B }).join('\n')).toMatch(/different key/);
  });

  it('notices a public key that is not one', () => {
    expect(problems(manifest(), { pubkey: 'UNSET: not yet' }).join('\n')).toMatch(/not a valid minisign key/);
  });

  it('notices a signature bound to another version', () => {
    expect(problems(manifest(), { version: '0.1.0-alpha.4' }).join('\n')).toMatch(
      /made for version 0.1.0-alpha.3/,
    );
  });

  it('fails a signature that records no version when the application requires one', () => {
    // A signature made without --app-version has no version in its comment.
    const text = Buffer.from(SIGNATURE, 'base64').toString('utf8');
    const stripped = Buffer.from(text.replace(/\tversion:[^\n]*/, '')).toString('base64');
    const m = manifest({ signatures: Object.fromEntries(PLATFORMS.map((p) => [p.key, stripped])) });
    expect(problems(m).join('\n')).toMatch(/does not record a version/);
    expect(problems(m, { requireSignedVersion: false })).toEqual([]);
  });

  it('is not a manifest at all', () => {
    expect(problems(null)).toEqual(['the manifest is not an object']);
    expect(problems({ version: VERSION }).join('\n')).toMatch(/platforms is missing/);
  });
});

describe('a local base address, for testing an update without publishing one', () => {
  const base = 'http://127.0.0.1:8787/';

  it('addresses every file under the base, and verifies against it', () => {
    const m = manifest({ baseUrl: base });
    expect(m.platforms['windows-x86_64'].url).toBe(
      `http://127.0.0.1:8787/Scuttle-${VERSION}-windows-x64-setup.exe`,
    );
    expect(problems(m, { baseUrl: base })).toEqual([]);
  });

  it('is never accepted for a real release, which has no base', () => {
    const m = manifest({ baseUrl: base });
    expect(problems(m).join('\n')).toMatch(/not https/);
  });
});

describe('manifestProblems against the artifacts themselves', () => {
  let dir;
  afterEach(() => dir && rmSync(dir, { recursive: true, force: true }));

  const stage = (contents = ARTIFACT, skip = null) => {
    dir = mkdtempSync(join(tmpdir(), 'scuttle-manifest-'));
    for (const platform of PLATFORMS) {
      if (platform.key !== skip) writeFileSync(join(dir, platform.file(VERSION)), contents);
    }
    return dir;
  };

  it('accepts artifacts that match their signatures', () => {
    expect(problems(manifest(), { artifacts: stage() })).toEqual([]);
  });

  it('refuses an artifact that is not there', () => {
    expect(problems(manifest(), { artifacts: stage(ARTIFACT, 'windows-x86_64') }).join('\n')).toMatch(
      /Scuttle-0.1.0-alpha.3-windows-x64-setup.exe is not in/,
    );
  });

  it('refuses an artifact that does not match its signature', () => {
    const found = problems(manifest(), { artifacts: stage(Buffer.from('a different file')) });
    expect(found).toHaveLength(3);
    expect(found.join('\n')).toMatch(/does not verify/);
  });
});

// ---- the application's own configuration ---------------------------------------------------------

describe('configProblems', () => {
  const good = () => ({
    pubkey: KEY_A,
    endpoints: ['https://github.com/acltabontabon/scuttle/releases/download/updater-channels/{{channel}}.json'],
    requireSignedVersion: true,
  });

  it('accepts a finished configuration', () => {
    expect(configProblems(good())).toEqual([]);
    expect(decodePublicKey(KEY_A).keyId).toHaveLength(8);
  });

  it('refuses the placeholder key, so nothing ships that cannot verify', () => {
    expect(configProblems({ ...good(), pubkey: 'UNSET: paste the key here' }).join('\n')).toMatch(
      /not a real public key yet/,
    );
  });

  it('refuses insecure or channel-less endpoints', () => {
    expect(configProblems({ ...good(), endpoints: ['http://example.test/{{channel}}.json'] }).join('\n')).toMatch(
      /not https/,
    );
    expect(configProblems({ ...good(), endpoints: ['https://example.test/latest.json'] }).join('\n')).toMatch(
      /\{\{channel\}\}/,
    );
    expect(configProblems({ ...good(), endpoints: [] }).join('\n')).toMatch(/no update endpoints/);
  });

  it('refuses to ship with the downgrade check off or with insecure transport allowed', () => {
    expect(configProblems({ ...good(), requireSignedVersion: false }).join('\n')).toMatch(/requireSignedVersion/);
    expect(configProblems({ ...good(), dangerousInsecureTransportProtocol: true }).join('\n')).toMatch(
      /dangerousInsecureTransportProtocol/,
    );
  });

  it('refuses when there is no updater section at all', () => {
    expect(configProblems(null)).toHaveLength(1);
  });
});
