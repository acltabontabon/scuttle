/**
 * The page works without any of this.
 *
 * Every download link is a real link in the markup, every section has its
 * words in the document, and the findings screen stands in for the animated
 * floor. This file brings the floor to life where it can and gets out of the
 * way when it cannot: each piece starts separately, one failure never stops
 * the next, and under reduced motion everything is simply drawn finished.
 */

import { setUpDownloads } from './downloads.js';

const still = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
const fine = window.matchMedia('(pointer: fine)').matches;

const clamp = (v, lo = 0, hi = 1) => Math.min(hi, Math.max(lo, v));
const lerp = (a, b, t) => a + (b - a) * t;
const easeOut = (t) => 1 - Math.pow(1 - t, 3);
const easeInOut = (t) => (t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2);
/** A small, repeatable pseudo-random number, so the floor looks the same twice. */
const rand = (seed) => {
  const x = Math.sin(seed * 12.9898 + 78.233) * 43758.5453;
  return x - Math.floor(x);
};

/* --------------------------------------------------------------------------
 * The drawn things. The same objects the app draws its piles with.
 * -------------------------------------------------------------------------- */

const GLYPHS = {
  installers: `<path d="M4 8.4 L14 3.4 L24 8.4 L24 18 L14 23 L4 18 Z"/><path d="M4 8.4 L14 13.4 L24 8.4"/><path d="M14 13.4 L14 23"/>`,
  ghosts: `<path d="M6 21 L6 10 C6 5.4 9.6 2.5 14 2.5 C18.4 2.5 22 5.4 22 10 L22 21 L18.6 18 L15.4 21 L12.2 18 L9 21 Z"/><circle cx="11.2" cy="10.6" r="1.3"/><circle cx="17" cy="10.6" r="1.3"/>`,
  copies: `<rect x="3.4" y="3.6" width="13.4" height="13.4" rx="1.6"/><rect x="9.6" y="8.6" width="13.4" height="13.4" rx="1.6"/>`,
  screenshots: `<rect x="3.5" y="5.5" width="15" height="11" rx="1.4"/><rect x="8" y="9" width="15" height="11" rx="1.4"/><path d="M11 15.5 L14.4 12.6 L17 14.8 L20 11.6"/>`,
  doc: `<rect x="5" y="2.5" width="18" height="21" rx="2"/><path d="M9 9 H19 M9 13 H19 M9 17 H15"/>`,
};
const KINDS = ['installers', 'ghosts', 'copies', 'screenshots'];
const svg = (kind, attrs = '') =>
  `<svg viewBox="0 0 28 26" ${attrs} aria-hidden="true" focusable="false">${GLYPHS[kind]}</svg>`;

/* --------------------------------------------------------------------------
 * One loop for everything that follows scrolling or the pointer. It only runs
 * while something asked for a frame, so an idle page costs nothing.
 * -------------------------------------------------------------------------- */

const scenes = [];
let frameAsked = false;
const pointer = { x: innerWidth / 2, y: innerHeight / 2, seen: false };

function ask() {
  if (frameAsked) return;
  frameAsked = true;
  requestAnimationFrame((now) => {
    frameAsked = false;
    let again = false;
    for (const scene of scenes) {
      try {
        if (scene(now)) again = true;
      } catch (error) {
        console.warn('a scene stumbled:', error);
      }
    }
    if (again) ask();
  });
}
addEventListener('scroll', ask, { passive: true });
addEventListener('resize', ask);
addEventListener(
  'pointermove',
  (event) => {
    pointer.x = event.clientX;
    pointer.y = event.clientY;
    pointer.seen = true;
    ask();
  },
  { passive: true },
);

/** How far through a pinned section the page has scrolled, 0 to 1. */
function progressThrough(section) {
  const rect = section.getBoundingClientRect();
  const span = rect.height - innerHeight;
  if (span <= 0) return clamp(-rect.top / Math.max(rect.height, 1));
  return clamp(-rect.top / span);
}

/* --------------------------------------------------------------------------
 * The thread along the top, and the masthead settling onto the paper.
 * -------------------------------------------------------------------------- */

function setUpThread() {
  const thread = document.getElementById('thread');
  if (!thread) return;
  scenes.push(() => {
    const max = document.documentElement.scrollHeight - innerHeight;
    thread.style.setProperty('--p', String(max > 0 ? scrollY / max : 0));
  });
}

function setUpMasthead() {
  const bar = document.getElementById('masthead');
  const sentinel = document.querySelector('.masthead-sentinel');
  if (!bar || !sentinel || !('IntersectionObserver' in window)) return;
  new IntersectionObserver(([entry]) => {
    bar.dataset.stuck = String(!entry.isIntersecting);
  }).observe(sentinel);
}

/* --------------------------------------------------------------------------
 * The headline, dropped onto the floor one letter at a time. The heading
 * keeps its words for assistive technology; the letters are decoration.
 * -------------------------------------------------------------------------- */

function setUpTitle() {
  const title = document.getElementById('hero-title');
  if (!title || still) return;
  title.setAttribute('aria-label', title.textContent.replace(/\s+/g, ' ').trim());
  let n = 0;
  const split = (node) => {
    const out = document.createDocumentFragment();
    for (const child of [...node.childNodes]) {
      if (child.nodeType === Node.TEXT_NODE) {
        for (const part of child.textContent.split(/(\s+)/)) {
          if (!part) continue;
          if (/^\s+$/.test(part)) {
            out.append(document.createTextNode(' '));
            continue;
          }
          const word = document.createElement('span');
          word.className = 'word';
          word.setAttribute('aria-hidden', 'true');
          for (const letter of part) {
            const ch = document.createElement('span');
            ch.className = 'ch';
            ch.textContent = letter;
            ch.style.setProperty('--d', `${120 + n * 28 + rand(n) * 80}ms`);
            ch.style.setProperty('--r', `${(rand(n + 9) - 0.5) * 50}deg`);
            n += 1;
            word.append(ch);
          }
          out.append(word);
        }
      } else if (child.nodeType === Node.ELEMENT_NODE) {
        const copy = child.cloneNode(false);
        copy.append(split(child));
        out.append(copy);
      }
    }
    return out;
  };
  const fragment = split(title);
  title.replaceChildren(fragment);
}

/* --------------------------------------------------------------------------
 * The hero's margins: things lying about, drifting away from the pointer.
 * -------------------------------------------------------------------------- */

function setUpStrewn() {
  const host = document.getElementById('strewn');
  if (!host) return;
  const items = Array.from({ length: 14 }, (_, n) => {
    const kind = [...KINDS, 'doc'][n % 5];
    const left = 56 + rand(n) * 40;
    const top = 4 + rand(n + 3) * 66;
    return { kind, left, top, size: 26 + rand(n + 5) * 34, tilt: (rand(n + 7) - 0.5) * 60, depth: 0.2 + rand(n + 11) * 0.9 };
  });
  host.innerHTML = items
    .map((item) => svg(item.kind, `width="${item.size}" height="${item.size}" style="left:${item.left}%;top:${item.top}%"`))
    .join('');
  const nodes = [...host.children];
  if (still) {
    nodes.forEach((node, n) => (node.style.transform = `rotate(${items[n].tilt}deg)`));
    return;
  }
  const hero = host.closest('.hero');
  let t0 = null;
  scenes.push((now) => {
    if (t0 === null) t0 = now;
    const rect = hero.getBoundingClientRect();
    if (rect.bottom < 0) return false;
    const px = (pointer.x / innerWidth - 0.5) * 2;
    const py = (pointer.y / innerHeight - 0.5) * 2;
    nodes.forEach((node, n) => {
      const item = items[n];
      const bob = Math.sin((now - t0) / 1600 + n) * 6 * item.depth;
      const dx = -px * 26 * item.depth;
      const dy = -py * 18 * item.depth + bob - scrollY * 0.25 * item.depth;
      node.style.transform = `translate(${dx}px, ${dy}px) rotate(${item.tilt + px * 8 * item.depth}deg)`;
    });
    return true;
  });
}

/* --------------------------------------------------------------------------
 * Scuttle, on the floor. It follows the pointer along the floor, looks at
 * it, and hops and says something when clicked. With nobody pointing, it
 * wanders now and then. Never under reduced motion: then it simply sits.
 * -------------------------------------------------------------------------- */

const LINES = [
  'Your Downloads folder has developed lore.',
  'That ZIP has been here longer than some friendships.',
  'I found things. I have questions.',
  'Nothing moved. I’m a crab, not a landlord.',
  'Filed under: future you’s problem.',
  'I only take what you’ve looked at.',
  'Everything comes back. That’s the rule.',
  'Eleven screenshots of the same thing? Bold.',
];

function setUpCritter() {
  const floor = document.getElementById('floor');
  const critter = document.getElementById('critter');
  const bubble = document.getElementById('bubble');
  const hint = document.getElementById('floor-hint');
  if (!floor || !critter) return;

  const width = 132;
  let x = 60;
  let target = 60;
  let facing = 1;
  let lastPointerAt = 0;
  let nextWander = performance.now() + 5000;
  let line = Math.floor(rand(Date.now() % 97) * LINES.length);
  let hideBubble = 0;

  const place = () => {
    critter.style.setProperty('--x', `${x}px`);
    critter.querySelector('.creature').style.setProperty('--face', String(facing));
  };
  const bounds = () => floor.getBoundingClientRect();

  const say = (text, ms = 3200) => {
    if (!bubble) return;
    bubble.textContent = text;
    bubble.style.marginLeft = '0px';
    bubble.dataset.on = 'true';
    // Keep the bubble on the screen when Scuttle is near an edge.
    const r = bubble.getBoundingClientRect();
    const shift = r.left < 8 ? 8 - r.left : r.right > innerWidth - 8 ? innerWidth - 8 - r.right : 0;
    bubble.style.marginLeft = `${shift}px`;
    clearTimeout(hideBubble);
    hideBubble = setTimeout(() => (bubble.dataset.on = 'false'), ms);
  };

  critter.addEventListener('click', () => {
    critter.dataset.hop = 'false';
    void critter.offsetWidth;
    critter.dataset.hop = 'true';
    critter.dataset.mood = 'wave';
    setTimeout(() => (critter.dataset.mood = ''), 1500);
    line = (line + 1) % LINES.length;
    say(LINES[line]);
    if (hint) hint.dataset.gone = 'true';
  });

  if (still) {
    x = bounds().width * 0.72;
    place();
    return;
  }

  // Arrive: scuttle in from the left edge, wave, and say hello.
  x = -width;
  place();
  target = Math.min(bounds().width * 0.62, 760);
  setTimeout(() => {
    critter.dataset.mood = 'wave';
    say('Oh. Hello.', 2400);
    setTimeout(() => (critter.dataset.mood = ''), 1400);
  }, 2100);

  let last = performance.now();
  scenes.push((now) => {
    const dt = Math.min(64, now - last);
    last = now;
    const rect = bounds();
    if (rect.bottom < -200 || rect.top > innerHeight + 200) return false;

    const pointerNear = pointer.seen && pointer.y > rect.top - innerHeight * 0.6 && now - lastPointerAt < 4000;
    if (pointer.seen && pointer.y > rect.top - innerHeight * 0.6) {
      target = clamp(pointer.x - rect.left - width / 2, 0, rect.width - width);
    } else if (!pointerNear && now > nextWander) {
      nextWander = now + 3500 + rand(now) * 4000;
      target = clamp(x + (rand(now + 1) - 0.5) * 500, 0, rect.width - width);
    }

    const distance = target - x;
    const speed = 0.38 * dt;
    if (Math.abs(distance) > 2) {
      x += Math.sign(distance) * Math.min(Math.abs(distance), speed * (0.4 + Math.min(1, Math.abs(distance) / 220)));
      facing = distance > 0 ? 1 : -1;
      critter.dataset.walking = 'true';
    } else {
      critter.dataset.walking = 'false';
    }
    place();

    // Eyes follow the pointer, a little.
    const cx = rect.left + x + width / 2;
    const cy = rect.bottom - 60;
    const angle = Math.atan2(pointer.y - cy, (pointer.x - cx) * facing);
    const reach = pointer.seen ? 2.4 : 0;
    const eyes = critter.querySelector('.eyes');
    eyes.style.setProperty('--lx', `${Math.cos(angle) * reach}px`);
    eyes.style.setProperty('--ly', `${Math.sin(angle) * reach}px`);
    return Math.abs(distance) > 2 || now < nextWander + 50;
  });

  addEventListener('pointermove', () => (lastPointerAt = performance.now()), { passive: true });
  // Keep it pottering even when nobody moves the mouse.
  setInterval(ask, 1200);
}

/* --------------------------------------------------------------------------
 * ONE: the rummage. A floor of ~110 loose things. As the section scrolls by,
 * a sweep passes over them, and each one travels, turning, onto its pile.
 * -------------------------------------------------------------------------- */

function setUpRummage() {
  const section = document.getElementById('rummage');
  const floor = document.getElementById('rummage-floor');
  const sweep = document.getElementById('sweep');
  const counter = document.getElementById('files-seen');
  const swaps = [...document.querySelectorAll('#rummage .swap')];
  if (!section || !floor) return;

  const setStep = (step) => swaps.forEach((s) => (s.dataset.on = String(Number(s.dataset.step) === step)));
  setStep(0);
  if (still) {
    setStep(2);
    if (counter) counter.textContent = '301,264';
    return;
  }

  floor.classList.add('alive');
  floor.querySelectorAll('.pile-labels span').forEach((s, i) => s.style.setProperty('--i', String(i)));
  const COUNT = innerWidth < 700 ? 64 : 112;
  const bits = Array.from({ length: COUNT }, (_, n) => {
    const pile = n % 4;
    const order = Math.floor(n / 4);
    const perPile = Math.ceil(COUNT / 4);
    return {
      kind: KINDS[pile],
      pile,
      sx: rand(n) * 0.92 + 0.02,
      sy: rand(n + 0.5) * 0.8 + 0.04,
      sr: (rand(n + 1.5) - 0.5) * 140,
      // Heaped: the first lie wide at the bottom, later ones land on top.
      h: order / perPile,
      jx: rand(n + 2.5) - 0.5,
      jy: rand(n + 3.5),
      er: (rand(n + 4.5) - 0.5) * 40,
      delay: rand(n + 5.5) * 0.35,
    };
  });
  floor.insertAdjacentHTML('beforeend', bits.map((b) => svg(b.kind, 'class="bit"')).join(''));
  const nodes = [...floor.querySelectorAll('svg.bit')];

  let settled = false;
  scenes.push(() => {
    const rect = section.getBoundingClientRect();
    if (rect.bottom < 0 || rect.top > innerHeight) return false;
    const p = progressThrough(section);
    const w = floor.clientWidth;
    const h = floor.clientHeight;
    const size = w < 500 ? 26 : 34;

    // 0.00–0.18 the sweep crosses; 0.18–0.8 things travel; after, they rest.
    const sweepT = clamp(p / 0.2);
    sweep.style.setProperty('--sx', `${lerp(-200, w + 40, sweepT)}px`);
    sweep.style.setProperty('--so', String(sweepT > 0 && sweepT < 1 ? 1 : 0));
    const travel = clamp((p - 0.16) / 0.62);
    setStep(p < 0.14 ? 0 : p < 0.5 ? 1 : 2);
    if (counter) counter.textContent = Math.round(easeOut(clamp(p / 0.8)) * 301264).toLocaleString('en-US');

    const pileWidth = w / 4;
    nodes.forEach((node, n) => {
      const b = bits[n];
      const own = clamp((travel - b.delay) / (1 - 0.35));
      const t = easeInOut(own);
      // A heap: wide at the floor, narrowing to a peak.
      const level = Math.pow(b.h, 0.75);
      const spread = 1 - level * 0.85;
      const ex = pileWidth * (b.pile + 0.5) + b.jx * pileWidth * 0.95 * spread - size / 2;
      const ey = h - 76 - level * (h * 0.24) - b.jy * 8 - size;
      const sx = b.sx * (w - size);
      const sy = b.sy * (h - size - 60);
      const arc = Math.sin(t * Math.PI) * -60;
      const x = lerp(sx, ex, t);
      const y = lerp(sy, ey, t) + arc;
      const r = lerp(b.sr, b.er, t);
      node.style.transform = `translate(${x}px, ${y}px) rotate(${r}deg) scale(${size / 34})`;
      const lit = sweepT > 0 && sweepT < 1 && Math.abs(sx - lerp(-200, w + 40, sweepT)) < 90;
      node.dataset.lit = String(lit || (t > 0.02 && t < 0.98));
    });

    const done = travel > 0.97;
    if (done !== settled) {
      settled = done;
      floor.dataset.settled = String(done);
    }
  });
}

/* --------------------------------------------------------------------------
 * TWO: the review. The path types itself, the caution is yours to tick, and
 * the button only wakes once you have. Then it goes in the drawer.
 * -------------------------------------------------------------------------- */

function setUpReview() {
  const box = document.getElementById('caution');
  const move = document.getElementById('sheet-move');
  const done = document.getElementById('sheet-done');
  const flyer = document.getElementById('flyer');
  const path = document.getElementById('sheet-path');
  if (!box || !move) return;

  box.addEventListener('change', () => (move.disabled = !box.checked));
  move.addEventListener('click', () => {
    move.disabled = true;
    box.disabled = true;
    if (flyer && !still) flyer.dataset.go = 'true';
    if (done) done.textContent = 'In the Drawer. Nothing deleted — put it back any time.';
    setTimeout(() => {
      if (flyer) flyer.dataset.go = 'false';
      box.checked = false;
      box.disabled = false;
      if (done) done.textContent = '';
    }, 4200);
  });

  if (path && !still && 'IntersectionObserver' in window) {
    const full = path.textContent;
    path.textContent = '';
    const watcher = new IntersectionObserver(([entry]) => {
      if (!entry.isIntersecting) return;
      watcher.disconnect();
      path.classList.add('typing');
      let i = 0;
      const tick = setInterval(() => {
        i += 1;
        path.textContent = full.slice(0, i);
        if (i >= full.length) {
          clearInterval(tick);
          setTimeout(() => path.classList.remove('typing'), 900);
        }
      }, 38);
    }, { threshold: 0.6 });
    watcher.observe(path);
  }

  // The finding card leans towards the pointer, a little.
  const card = document.querySelector('.tilt');
  if (card && fine && !still) {
    card.addEventListener('pointermove', (event) => {
      const r = card.getBoundingClientRect();
      const x = (event.clientX - r.left) / r.width - 0.5;
      const y = (event.clientY - r.top) / r.height - 0.5;
      card.style.transform = `rotateY(${x * 10}deg) rotateX(${-y * 8}deg) translateZ(10px)`;
    });
    card.addEventListener('pointerleave', () => (card.style.transform = ''));
  }
  document.querySelectorAll('.app-dome').forEach((dome, i) => dome.style.setProperty('--i', String(i)));
}

/* --------------------------------------------------------------------------
 * THREE: the drawer. The page goes dark, the drawer slides out as you scroll,
 * things drop in, and near the end one of them is put back.
 * -------------------------------------------------------------------------- */

function setUpDrawer() {
  const section = document.getElementById('drawer-section');
  const drawer = document.getElementById('drawer3d');
  const host = document.getElementById('drawer-items');
  if (!section || !drawer || !host) return;

  const ITEMS = [
    { kind: 'installers', x: 0.12 }, { kind: 'doc', x: 0.28 }, { kind: 'screenshots', x: 0.44 },
    { kind: 'copies', x: 0.6 }, { kind: 'ghosts', x: 0.76 }, { kind: 'doc', x: 0.36 },
  ];
  host.innerHTML = ITEMS.map((item) => svg(item.kind)).join('');
  const nodes = [...host.children];

  if (still) {
    drawer.style.setProperty('--open', '1');
    nodes.forEach((node, n) => (node.style.transform = `translate(${ITEMS[n].x * 440}px, ${120 - (n % 2) * 26}px) rotate(${(rand(n) - 0.5) * 30}deg)`));
    return;
  }

  scenes.push(() => {
    const rect = section.getBoundingClientRect();
    if (rect.bottom < 0 || rect.top > innerHeight) return false;
    const p = progressThrough(section);
    const open = easeOut(clamp((p - 0.05) / 0.35));
    drawer.style.setProperty('--open', open.toFixed(4));
    const inside = host.clientWidth;
    const depth = host.clientHeight;
    nodes.forEach((node, n) => {
      const start = 0.3 + n * 0.06;
      const fall = easeOut(clamp((p - start) / 0.14));
      // The last one comes back out: "Put it back".
      const back = n === ITEMS.length - 1 ? easeInOut(clamp((p - 0.82) / 0.14)) : 0;
      const x = ITEMS[n].x * (inside - 40);
      const rest = depth - 44 - (n % 2) * 22;
      const y = lerp(-120, rest, fall) - back * (depth + 260);
      const r = (rand(n) - 0.5) * 40 * fall + back * 25;
      node.style.opacity = String(fall > 0 ? 1 : 0);
      node.style.transform = `translate(${x}px, ${y}px) rotate(${r}deg)`;
    });
  });
}

/* --------------------------------------------------------------------------
 * Rooms. A section can change the tone of the whole page while it is in the
 * middle of the screen — the drawer turns the lights down.
 * -------------------------------------------------------------------------- */

function setUpTones() {
  const toned = [...document.querySelectorAll('main [data-tone]')];
  if (!toned.length || !('IntersectionObserver' in window)) return;
  const active = new Set();
  const watcher = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (entry.isIntersecting) active.add(entry.target);
        else active.delete(entry.target);
      }
      document.body.dataset.tone = active.size ? [...active][0].dataset.tone : 'paper';
    },
    // Only while the section holds the middle of the screen.
    { rootMargin: '-45% 0px -45% 0px' },
  );
  toned.forEach((el) => watcher.observe(el));
}

/* --------------------------------------------------------------------------
 * FOUR: the burrow. A pretend menu bar with the real icon. Click it.
 * -------------------------------------------------------------------------- */

const BURROW_LINES = [
  'I found things. I have questions.',
  'The drawer is empty. Suspiciously responsible.',
  'Nothing moved. I’m a crab, not a landlord.',
  'Filed under: future you’s problem.',
];

function setUpBurrow() {
  const tray = document.getElementById('tray');
  const pop = document.getElementById('burrow-pop');
  const line = document.getElementById('pop-line');
  const desk = tray?.closest('.desk');
  const clock = document.getElementById('clock');
  if (!tray || !pop) return;
  let n = 0;
  const set = (open) => {
    pop.dataset.open = String(open);
    tray.setAttribute('aria-expanded', String(open));
    if (open) {
      if (desk) desk.dataset.found = 'true';
      if (line) line.textContent = BURROW_LINES[n++ % BURROW_LINES.length];
      // Replay the peek.
      const critter = pop.querySelector('.pop-critter');
      critter.style.animation = 'none';
      void critter.offsetWidth;
      critter.style.animation = '';
    }
  };
  tray.addEventListener('click', (event) => {
    event.stopPropagation();
    set(pop.dataset.open !== 'true');
  });
  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && pop.dataset.open === 'true') {
      set(false);
      tray.focus();
    }
  });
  desk?.addEventListener('click', (event) => {
    if (!pop.contains(event.target)) set(false);
  });
  if (clock) {
    const now = new Date();
    clock.textContent = now.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
  }
}

/* --------------------------------------------------------------------------
 * The real thing: a window that straightens up as it arrives, and the demo.
 * -------------------------------------------------------------------------- */

function setUpShowcase() {
  const win = document.getElementById('window3d');
  if (!win) return;
  if (still) return;
  scenes.push(() => {
    const rect = win.getBoundingClientRect();
    if (rect.bottom < 0 || rect.top > innerHeight) return false;
    const t = clamp((rect.top - innerHeight * 0.18) / (innerHeight * 0.7));
    win.style.setProperty('--t', t.toFixed(4));
  });
}

function setUpDemo() {
  const figure = document.getElementById('hero-figure');
  const play = document.getElementById('hero-play');
  const label = document.getElementById('hero-play-label');
  const caption = document.getElementById('hero-caption');
  if (!figure || !play) return;
  const poster = document.getElementById('hero-media');
  const source = './media/demo.mp4';

  // Only offered once we know the recording is there. Nothing plays until
  // somebody asks, reduced motion or not.
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
      video.addEventListener('play', () => show(true));
      video.addEventListener('pause', () => show(false));
      figure.insertBefore(video, play);
      poster?.remove();
      if (caption) caption.textContent = 'Recorded from the running app against invented demonstration files. No real file was touched.';
    }
    if (video.paused) video.play().catch(() => show(false));
    else video.pause();
  });
}

/* --------------------------------------------------------------------------
 * The closing heap: things that shuffle away from the pointer, and jump when
 * somebody reaches for the download.
 * -------------------------------------------------------------------------- */

function setUpHeap() {
  const host = document.getElementById('closing-heap');
  const section = document.getElementById('get');
  if (!host || !section) return;
  const COUNT = innerWidth < 700 ? 28 : 54;
  const items = Array.from({ length: COUNT }, (_, n) => {
    const h = rand(n + 0.3);
    const centre = 0.5 + (rand(n) - 0.5) * (1.1 - h * 0.7);
    return { kind: [...KINDS, 'doc'][n % 5], x: centre, y: h * 120 + rand(n + 2) * 16, r: (rand(n + 4) - 0.5) * 70 };
  });
  host.innerHTML = items.map((item) => svg(item.kind)).join('');
  const nodes = [...host.children];
  let jump = 0;
  const draw = (now) => {
    const rect = host.getBoundingClientRect();
    const width = rect.width;
    const kick = jump > now ? Math.sin(((jump - now) / 700) * Math.PI) : 0;
    nodes.forEach((node, n) => {
      const item = items[n];
      let x = item.x * width - 22;
      let y = -item.y;
      if (!still && pointer.seen) {
        const dx = x + 22 - (pointer.x - rect.left);
        const dy = rect.bottom - item.y - pointer.y;
        const d = Math.hypot(dx, dy);
        if (d < 160) {
          const push = (1 - d / 160) * 40;
          x += (dx / (d || 1)) * push;
          y += (dy / (d || 1)) * push * 0.6;
        }
      }
      y -= kick * (30 + rand(n) * 60);
      node.style.transform = `translate(${x}px, ${y}px) rotate(${item.r + kick * 40 * (rand(n + 1) - 0.5)}deg)`;
    });
    return jump > now;
  };
  draw(performance.now());
  if (still) return;
  scenes.push((now) => {
    const rect = section.getBoundingClientRect();
    if (rect.bottom < 0 || rect.top > innerHeight) return false;
    return draw(now);
  });
  document.getElementById('download-secondary')?.addEventListener('pointerenter', () => {
    jump = performance.now() + 700;
    ask();
  });
}

/* --------------------------------------------------------------------------
 * Buttons that lean towards the pointer.
 * -------------------------------------------------------------------------- */

function setUpMagnets() {
  if (still || !fine) return;
  for (const button of document.querySelectorAll('.magnetic')) {
    button.addEventListener('pointermove', (event) => {
      const r = button.getBoundingClientRect();
      const x = event.clientX - (r.left + r.width / 2);
      const y = event.clientY - (r.top + r.height / 2);
      button.style.transform = `translate(${x * 0.18}px, ${y * 0.28}px)`;
    });
    button.addEventListener('pointerleave', () => (button.style.transform = ''));
  }
}

/* --------------------------------------------------------------------------
 * Arriving. `.reveal` in the markup, `.revealed` from here — and a stop, so
 * nothing ever stays invisible because an animation did not fire.
 * -------------------------------------------------------------------------- */

function setUpReveals() {
  const targets = [...document.querySelectorAll('.reveal')];
  document.querySelectorAll('.hero .reveal').forEach((el, i) => el.style.setProperty('--delay', `${900 + i * 140}ms`));
  document.querySelectorAll('.reasons li').forEach((li, i) => li.style.setProperty('--i', String(i)));
  if (still || !('IntersectionObserver' in window)) {
    targets.forEach((t) => t.classList.add('revealed'));
    return;
  }
  const watcher = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        entry.target.classList.add('revealed');
        watcher.unobserve(entry.target);
      }
    },
    { rootMargin: '0px 0px -8% 0px', threshold: 0 },
  );
  targets.forEach((t) => watcher.observe(t));
  // The hero is above the fold: readable now, whatever the observer does.
  setTimeout(() => document.querySelectorAll('.hero .reveal').forEach((t) => t.classList.add('revealed')), 50);
}

for (const start of [
  setUpTitle,
  setUpThread,
  setUpMasthead,
  setUpReveals,
  setUpStrewn,
  setUpCritter,
  setUpRummage,
  setUpReview,
  setUpDrawer,
  setUpTones,
  setUpBurrow,
  setUpShowcase,
  setUpDemo,
  setUpHeap,
  setUpMagnets,
  setUpDownloads,
]) {
  try {
    start();
  } catch (error) {
    // One broken flourish must not take the page down with it.
    console.warn(`${start.name} did not start:`, error);
  }
}
ask();
