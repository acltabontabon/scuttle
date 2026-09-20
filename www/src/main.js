/**
 * The page works without any of this.
 *
 * Every download link is a real link in the markup, the drawer's contents are
 * an ordinary image in the document, and the piles are decorative. This file
 * improves those things where it can and gets out of the way when it cannot,
 * which is why each piece is started separately and a failure in one never
 * stops the next.
 */

import { setUpDownloads } from './downloads.js';

const stillness = window.matchMedia('(prefers-reduced-motion: reduce)');

/* --------------------------------------------------------------------------
 * The piles.
 *
 * The same drawn objects the application uses — deliberately imperfect, with
 * a small deterministic wobble so two screenshots in a pile are not clones.
 * Copied rather than imported, because pulling the app's React components into
 * a marketing page would mean shipping React to read four paragraphs.
 * -------------------------------------------------------------------------- */

const GLYPHS = {
  ghosts: (w) =>
    `<path d="M6 21 L6 ${10 + w * 0.2} C6 5.4 9.6 2.5 14 2.5 C18.4 2.5 22 5.4 22 ${10 + w * 0.2} L22 21 L18.6 18 L15.4 21 L12.2 18 L9 21 Z"/>
     <circle cx="11.2" cy="10.6" r="1.5" fill="currentColor" stroke="none"/>
     <circle cx="17" cy="10.6" r="1.5" fill="currentColor" stroke="none"/>`,
  screenshots: (w) =>
    `<rect x="3.5" y="${5.5 + w * 0.2}" width="15" height="11" rx="1.4"/>
     <rect x="8" y="9" width="15" height="11" rx="1.4"/>
     <path d="M11 15.5 L14.4 12.6 L17 14.8 L20 11.6"/>`,
  installers: (w) =>
    `<path d="M4 8.4 L14 3.4 L24 8.4 L24 18 L14 ${23 + w * 0.1} L4 18 Z"/>
     <path d="M4 8.4 L14 13.4 L24 8.4"/><path d="M14 13.4 L14 23"/>`,
  copies: (w) =>
    `<rect x="3.4" y="3.6" width="13.4" height="13.4" rx="1.6"/>
     <rect x="9.6" y="${8.4 + w * 0.15}" width="13.4" height="13.4" rx="1.6"/>`,
};

const PILES = [
  { kind: 'ghosts', label: 'Leftovers', count: 6 },
  { kind: 'screenshots', label: 'Screenshots', count: 9 },
  { kind: 'installers', label: 'Installers', count: 7 },
  { kind: 'copies', label: 'Copies', count: 6 },
];

/** A small, repeatable pseudo-random number, so the piles look the same twice. */
function scatter(seed) {
  const x = Math.sin(seed * 12.9898) * 43758.5453;
  return x - Math.floor(x);
}

function drawPiles() {
  const host = document.getElementById('piles');
  if (!host) return;

  host.innerHTML = PILES.map((pile, pileIndex) => {
    const objects = Array.from({ length: pile.count }, (_, n) => {
      const seed = pileIndex * 17 + n;
      // A heap, not a scatter: the first objects lie flat and wide at the
      // bottom and later ones land on top of them, so the shape reads as a
      // pile someone emptied out rather than as icons floating in a box.
      const height = n / Math.max(pile.count - 1, 1);
      const spread = 1 - height * 0.55;
      const left = 50 + (scatter(seed) - 0.5) * 86 * spread - 14;
      const bottom = height * 52 + scatter(seed + 0.5) * 12;
      const tilt = (scatter(seed + 1.5) - 0.5) * 34;
      const delay = (pileIndex * 5 + n) * 45;
      return `<svg viewBox="0 0 28 26" width="30" height="28" fill="none" stroke="currentColor"
                   stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"
                   style="left:${left}%;bottom:${bottom}px;transform:rotate(${tilt}deg);animation-delay:${delay}ms">
                ${GLYPHS[pile.kind](((n * 37) % 7) - 3)}
              </svg>`;
    }).join('');

    return `<div class="pile"><span class="pile-shadow"></span>${objects}<span class="pile-label">${pile.label}</span></div>`;
  }).join('');
}

/* --------------------------------------------------------------------------
 * The drawer.
 *
 * It opens on a button and on the keyboard, because it is a button. Dragging
 * is never required and never offered — the metaphor is in the drawing, not
 * in the gesture.
 * -------------------------------------------------------------------------- */

function setUpDrawer() {
  const drawer = document.getElementById('drawer');
  const toggle = document.getElementById('drawer-toggle');
  const body = document.getElementById('drawer-body');
  if (!drawer || !toggle || !body) return;

  const set = (open) => {
    drawer.dataset.open = String(open);
    toggle.setAttribute('aria-expanded', String(open));
    toggle.textContent = open ? 'Close the drawer' : 'Open the drawer';
  };

  set(false);
  toggle.addEventListener('click', () => set(drawer.dataset.open !== 'true'));
}

/* --------------------------------------------------------------------------
 * The demo.
 *
 * The poster is in the markup and is what everybody gets first: a real
 * screenshot, not a placeholder. The loop is only fetched when someone asks
 * for it, so nobody downloads several megabytes to read a paragraph, and it
 * is never offered at all to a visitor who has asked for less motion.
 * -------------------------------------------------------------------------- */

function setUpDemo() {
  const figure = document.getElementById('hero-figure');
  const play = document.getElementById('hero-play');
  const label = document.getElementById('hero-play-label');
  const caption = document.getElementById('hero-caption');
  if (!figure || !play) return;

  const poster = document.getElementById('hero-media');
  const source = './media/demo.mp4';

  // Only offer it once we know it is there. A missing recording should leave
  // a clean screenshot rather than a button that does nothing.
  //
  // The offer stands under reduced motion too. Asking for less motion means
  // "do not move things at me", not "hide the video" — nothing plays until
  // somebody presses the button, and taking the button away would remove the
  // choice rather than respect it.
  fetch(source, { method: 'HEAD' })
    .then((response) => {
      if (response.ok) play.hidden = false;
    })
    .catch(() => {});

  let video = null;

  const show = (playing) => {
    play.dataset.playing = String(playing);
    if (label) label.textContent = playing ? 'Pause' : 'Play the demo';
  };

  play.addEventListener('click', () => {
    if (!video) {
      video = document.createElement('video');
      video.src = source;
      video.muted = true;
      video.loop = true;
      video.playsInline = true;
      video.setAttribute('aria-label', poster?.alt ?? 'A run through Scuttle');
      // The button reports what the video is actually doing rather than what
      // it was asked to do: play() can be refused — a background tab, a power
      // saving rule — and a button reading "Pause" over a still frame is
      // worse than no button at all.
      video.addEventListener('play', () => show(true));
      video.addEventListener('pause', () => show(false));
      figure.insertBefore(video, play);
      poster?.remove();
      if (caption) {
        caption.textContent =
          'Recorded from the running app against invented demonstration files. No real file was touched.';
      }
    }
    if (video.paused) {
      video.play().catch(() => show(false));
    } else {
      video.pause();
    }
  });
}

/* --------------------------------------------------------------------------
 * Arriving.
 *
 * Sections lift into place as they come into view, and the piles only drop
 * once somebody is actually looking at them. `.reveal` is applied by the
 * markup and `.revealed` by this: with JavaScript off, or with reduced motion,
 * the stylesheet leaves everything in its finished state, so nothing here is
 * load-bearing.
 * -------------------------------------------------------------------------- */

/**
 * The masthead settles onto the paper once the top of the page is behind you.
 *
 * Watched through a one-pixel sentinel rather than a scroll listener, so
 * there is no handler running on every frame of every scroll.
 */
function setUpMasthead() {
  const bar = document.getElementById('masthead');
  const sentinel = document.querySelector('.masthead-sentinel');
  if (!bar || !sentinel || !('IntersectionObserver' in window)) return;

  new IntersectionObserver(
    ([entry]) => {
      bar.dataset.stuck = String(!entry.isIntersecting);
    },
    { threshold: 0 },
  ).observe(sentinel);
}

function setUpReveals() {
  const targets = document.querySelectorAll('.reveal, .piles, .stagger');
  if (stillness.matches || !('IntersectionObserver' in window)) {
    for (const target of targets) target.classList.add('revealed');
    return;
  }

  const watcher = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        // The piles live inside a beat; mark the beat too, so the pile's own
        // keyframes are scoped to the moment it appears.
        (entry.target.closest('.beat') ?? entry.target).classList.add('revealed');
        entry.target.classList.add('revealed');
        watcher.unobserve(entry.target);
      }
    },
    // A threshold of zero, because a section taller than the window can never
    // reach a large intersection ratio, and a paragraph that stays invisible
    // because an animation did not fire is not a paragraph.
    { rootMargin: '0px 0px -6% 0px', threshold: 0 },
  );
  for (const target of targets) watcher.observe(target);

  // And a stop: whatever happens above — a browser quirk, a section that
  // never quite intersects, a jump straight to an anchor — everything is
  // visible shortly after the page settles. The animation is a nicety; the
  // words are not.
  // Short, because the hero is inside this: anything above the fold has to be
  // readable now, not in a moment. The observer will normally have got there
  // first, and adding the class twice costs nothing.
  window.setTimeout(() => {
    for (const target of targets) target.classList.add('revealed');
  }, 1200);
}

/* --------------------------------------------------------------------------
 * The things lying about in the hero's margin.
 *
 * Purely scenery — the same drawn objects as the piles, at a tenth of their
 * weight, so the top of the page reads as a floor somebody has been rummaging
 * on rather than as an empty rectangle.
 * -------------------------------------------------------------------------- */

const STREWN = [
  { kind: 'ghosts', left: 1, top: 16, size: 44, tilt: -14 },
  { kind: 'installers', left: 8, top: 72, size: 52, tilt: 9 },
  { kind: 'screenshots', left: 90, top: 6, size: 48, tilt: 12 },
  { kind: 'copies', left: 93, top: 62, size: 40, tilt: -8 },
  { kind: 'ghosts', left: 80, top: 88, size: 36, tilt: 17 },
  { kind: 'installers', left: 33, top: 94, size: 34, tilt: -11 },
];

function drawStrewn() {
  const host = document.getElementById('strewn');
  if (!host) return;
  host.innerHTML = STREWN.map(
    (item, n) => `<svg viewBox="0 0 28 26" width="${item.size}" height="${item.size}" fill="none"
        stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"
        style="left:${item.left}%;top:${item.top}%;transform:rotate(${item.tilt}deg);animation-delay:${n * 70}ms">
        ${GLYPHS[item.kind](((n * 37) % 7) - 3)}
      </svg>`,
  ).join('');
}

for (const start of [
  drawPiles,
  drawStrewn,
  setUpMasthead,
  setUpReveals,
  setUpDrawer,
  setUpDemo,
  setUpDownloads,
]) {
  try {
    start();
  } catch (error) {
    // One broken flourish must not take the page down with it.
    console.warn(`${start.name} did not start:`, error);
  }
}
