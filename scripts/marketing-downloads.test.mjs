import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { setUpDownloads } from '../www/src/downloads.js';

const html = readFileSync(resolve(process.cwd(), 'www/index.html'), 'utf8');
const release = {
  tag_name: 'v0.1.0', draft: false, prerelease: false,
  html_url: 'https://github.com/acltabontabon/scuttle/releases/tag/v0.1.0',
  assets: [
    { name: 'Scuttle-0.1.0-macos-apple-silicon.dmg', browser_download_url: 'https://example.com/mac.dmg', size: 1024 },
    { name: 'Scuttle-0.1.0-windows-x64-setup.exe', browser_download_url: 'https://example.com/windows.exe', size: 2048 },
  ],
};
beforeEach(() => {
  document.body.innerHTML = html.slice(html.indexOf('<body'), html.indexOf('</body>') + 7);
  vi.stubGlobal('fetch', vi.fn(async () => ({ ok: true, json: async () => release })));
});
afterEach(() => { document.body.innerHTML = ''; vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const primary = () => document.getElementById('download-primary');

it('offers desktop release choices on a phone instead of a Mac installer', async () => {
  vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue('iPhone');
  await setUpDownloads();
  expect(primary().href).toBe(release.html_url);
  expect(document.getElementById('download-primary-note').textContent).toBe('Desktop app · macOS & Windows');
});

it('does not offer a Mac installer when a Windows build is missing', async () => {
  vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue('Windows NT');
  fetch.mockResolvedValue({ ok: true, json: async () => ({ ...release, assets: [release.assets[0]] }) });
  await setUpDownloads();
  expect(primary().href).toBe(release.html_url);
  expect(document.getElementById('download-primary-label').textContent).toBe('View available builds');
});

it('uses the matching desktop installer and keeps other systems available', async () => {
  vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue('Windows NT');
  await setUpDownloads();
  expect(primary().href).toBe('https://example.com/windows.exe');
  expect(document.getElementById('download-list').querySelector('[data-slug="macos-apple-silicon"]').href).toBe('https://example.com/mac.dmg');
});

it('keeps the original release links when the API is unavailable', async () => {
  fetch.mockRejectedValue(new Error('offline'));
  await setUpDownloads();
  expect(primary().href).toBe('https://github.com/acltabontabon/scuttle/releases/latest');
});
