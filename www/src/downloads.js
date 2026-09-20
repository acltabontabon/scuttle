/**
 * Working download links, or an honest statement that there are none yet.
 *
 * The markup already ships a real link to the releases page, so this only
 * ever improves on something that works. Every failure path — no network,
 * a rate-limited API, a release with the wrong assets, no release at all —
 * leaves that link exactly as it was rather than replacing it with something
 * broken.
 *
 * No token is used and none is needed: the unauthenticated endpoint is public
 * and rate-limited per IP, which is fine for one request per visit.
 */

const REPO = 'acltabontabon/scuttle';
const RELEASES = `https://github.com/${REPO}/releases`;

/**
 * The three things a release is expected to contain, in the order someone
 * scanning the list would want them.
 */
const TARGETS = [
  {
    slug: 'macos-apple-silicon',
    os: 'macOS',
    detail: 'Apple silicon — M1 and later',
    suffix: '.dmg',
  },
  { slug: 'macos-intel', os: 'macOS', detail: 'Intel', suffix: '.dmg' },
  { slug: 'windows-x64-setup', os: 'Windows', detail: '10 or 11, 64-bit', suffix: '.exe' },
];

/**
 * A guess at which row to point at first — never more than a guess.
 *
 * A browser will tell you it is on a Mac. It will not reliably tell you
 * whether that Mac is Apple silicon or Intel: Safari reports the same platform
 * for both, and Chrome's user-agent client hints report Intel on an Apple
 * silicon machine running under Rosetta. So the Mac default is Apple silicon
 * because that is what almost every Mac sold since 2020 is, the row says so in
 * words, and the other rows are one click away and always visible.
 */
function likelyTarget() {
  const platform = `${navigator.userAgentData?.platform ?? ''} ${navigator.platform ?? ''} ${navigator.userAgent}`;
  if (/Win/i.test(platform)) return 'windows-x64-setup';
  if (/Mac|iPhone|iPad/i.test(platform)) return 'macos-apple-silicon';
  return null;
}

/** The newest published, non-draft, non-prerelease release. */
async function stableRelease() {
  // `/releases/latest` is already defined as the latest non-prerelease, and
  // returns 404 when a repository has only prereleases or none at all — which
  // is exactly the distinction wanted here.
  const response = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`, {
    headers: { Accept: 'application/vnd.github+json' },
  });
  if (!response.ok) return null;

  const release = await response.json();
  if (!release || release.draft || release.prerelease) return null;
  return release;
}

/**
 * The newest prerelease, for when there is no stable one yet.
 *
 * Offered, but never as though it were finished: the button says so, and it
 * is only ever reached when `stableRelease` has already come back empty. A
 * prerelease presented as a release is the one thing this must not do.
 */
async function newestPrerelease() {
  const response = await fetch(`https://api.github.com/repos/${REPO}/releases?per_page=10`, {
    headers: { Accept: 'application/vnd.github+json' },
  });
  if (!response.ok) return null;

  const releases = await response.json();
  if (!Array.isArray(releases)) return null;
  // The API returns them newest first.
  return releases.find((release) => release && !release.draft && release.prerelease) ?? null;
}

function assetsFor(release) {
  const found = new Map();
  for (const target of TARGETS) {
    const asset = (release.assets ?? []).find(
      (candidate) =>
        typeof candidate.name === 'string' &&
        candidate.name.includes(target.slug) &&
        candidate.name.endsWith(target.suffix),
    );
    if (asset?.browser_download_url) found.set(target.slug, asset);
  }
  return found;
}

function megabytes(bytes) {
  return typeof bytes === 'number' && bytes > 0 ? `${(bytes / 1024 / 1024).toFixed(1)} MB` : '';
}

function say(element, text) {
  if (element) element.textContent = text;
}

export async function setUpDownloads() {
  const root = document.getElementById('download');
  const primary = document.getElementById('download-primary');
  const label = document.getElementById('download-primary-label');
  const note = document.getElementById('download-primary-note');
  const list = document.getElementById('download-list');
  const secondary = document.getElementById('download-secondary');
  if (!root || !primary || !list) return;

  const rows = new Map(
    [...list.querySelectorAll('a[data-slug]')].map((anchor) => [anchor.dataset.slug, anchor]),
  );

  let release = null;
  let early = false;
  try {
    release = await stableRelease();
    if (!release) {
      release = await newestPrerelease();
      early = Boolean(release);
    }
  } catch {
    // Offline, blocked, or rate-limited. The static links stay.
    return;
  }

  if (!release) {
    // No stable release yet. Say so plainly rather than offering a download
    // that would 404, and leave every link pointing at the releases page.
    root.dataset.state = 'none';
    say(label, 'No builds published yet');
    say(note, 'watch the repository for the first release');
    primary.href = RELEASES;
    if (secondary) {
      secondary.href = RELEASES;
      secondary.textContent = 'Follow along on GitHub';
    }
    const warning = document.createElement('p');
    warning.className = 'download-warning';
    warning.textContent =
      'No release yet. You can build it from source in the meantime — the README has the commands.';
    root.append(warning);
    return;
  }

  const assets = assetsFor(release);
  if (assets.size === 0) return; // A release, but not one we recognise. Leave it be.

  const version = (release.tag_name ?? '').replace(/^v/, '');

  for (const target of TARGETS) {
    const row = rows.get(target.slug);
    const asset = assets.get(target.slug);
    if (!row) continue;
    if (!asset) {
      // A target this release does not contain. Point it at the release page
      // rather than inventing a URL, and say what happened.
      row.href = release.html_url ?? RELEASES;
      const detail = row.querySelector('span');
      if (detail) detail.textContent = `${target.detail} — not in ${release.tag_name}`;
      continue;
    }
    row.href = asset.browser_download_url;
    const detail = row.querySelector('span');
    if (detail) detail.textContent = [target.detail, megabytes(asset.size)].filter(Boolean).join(' · ');
  }

  const guess = likelyTarget();
  const chosen = (guess && assets.get(guess)) || assets.values().next().value;
  const chosenTarget = TARGETS.find((target) => assets.get(target.slug) === chosen);

  if (chosen && chosenTarget) {
    root.dataset.state = early ? 'early' : 'ready';
    primary.href = chosen.browser_download_url;
    say(label, early ? `Try the alpha on ${chosenTarget.os}` : `Download for ${chosenTarget.os}`);
    say(
      note,
      [version && `${version}`, chosenTarget.detail, megabytes(chosen.size)]
        .filter(Boolean)
        .join(' · '),
    );
    rows.get(chosenTarget.slug)?.setAttribute('aria-current', 'true');
    if (secondary) {
      secondary.href = chosen.browser_download_url;
      if (early) secondary.textContent = 'Try the alpha';
    }

    if (early) {
      // Said in words next to the button, not only in the version number.
      const aside = document.createElement('p');
      aside.className = 'download-warning';
      aside.innerHTML =
        'This is an early build. It does what the page describes and its tests pass on both systems, ' +
        'but it has not been run on many machines yet — so keep an eye on what you empty from the drawer. ' +
        `<a href="${release.html_url ?? RELEASES}">What is in it</a>.`;
      root.append(aside);
    }
  }
}
