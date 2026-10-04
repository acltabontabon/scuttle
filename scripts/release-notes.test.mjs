import { describe, expect, it } from 'vitest';
import { changelogSection, releaseBody } from './release-notes.mjs';

describe('release notes', () => {
  it('extracts only the requested release, joins wrapped bullets and excludes link definitions', () => {
    const changelog = '## [Unreleased]\nNext release\n\n## [0.1.1] - 2026-10-05\n\n- A fix\n  with evidence.\n\n## [0.1.0]\nOld release\n\n[0.1.0]: https://example.com\n';
    expect(changelogSection(changelog, '0.1.1')).toBe('- A fix with evidence.');
    expect(changelogSection(changelog, '0.1.0')).toBe('Old release');
    expect(changelogSection(changelog, '9.9.9')).toBeNull();
  });

  it('pins all installer links, media and guides to the same version', () => {
    const body = releaseBody('0.1.1', 'Release notes', { media: 'demo.gif' });
    for (const file of ['macos-apple-silicon.dmg', 'macos-intel.dmg', 'windows-x64-setup.exe']) {
      expect(body).toContain(`/releases/download/v0.1.1/Scuttle-0.1.1-${file}`);
    }
    expect(body).toContain('/v0.1.1/docs/media/demo.gif');
    expect(body).toContain('/blob/v0.1.1/README.md');
    expect(body).not.toContain('/blob/main/');
    expect(releaseBody('0.1.1', 'Release notes')).not.toContain('/docs/media/');
  });

  it('retains important installation and drawer information in the polished page', () => {
    const body = releaseBody('0.1.1', 'Release notes');
    for (const text of ['Open Anyway', 'Run anyway', '**damaged**', 'without an Apple Developer ID or notarization', 'does not free space', 'Emptying the drawer is permanent', 'may expire']) {
      expect(body).toContain(text);
    }
  });

  it('keeps highlights visible while preserving detailed fixes and checksums', () => {
    const section = '**Steadier scans.**\n\n### Highlights\n\n- Scans finish.\n\n### Fixed\n\n- Database failure recovery.\n\n### Changed\n\n- Updated docs.';
    const body = releaseBody('0.1.1', section, { checksums: 'abc  installer.exe\n' });
    const details = body.indexOf('<summary><strong>All fixes & improvements');
    expect(body.indexOf('- Scans finish.')).toBeLessThan(details);
    expect(body.indexOf('- Database failure recovery.')).toBeGreaterThan(details);
    expect(body).toContain('- Updated docs.');
    expect(body).toContain('```text\nabc  installer.exe\n```');
    expect(releaseBody('0.1.1', '### Fixed\n- A fix.')).toContain('### Fixed\n- A fix.');
  });
});
