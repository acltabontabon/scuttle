#!/usr/bin/env node
/**
 * Generates the tray glyphs from the mascot.
 *
 * Run by hand, not by the build: the PNGs it writes are committed, so neither
 * `npm install` nor CI needs a rasteriser.
 *
 *   node scripts/tray-icons.mjs
 *
 * Why the shape is described here rather than traced from `Scuttle.tsx`: at
 * sixteen pixels the mascot's six legs land on top of each other and the whole
 * thing reads as a smudge. This is the same creature — a domed shell over legs
 * reaching down and out — drawn with a leg count and a stroke weight that
 * survive the size. Everything is supersampled and box-filtered down, which is
 * what keeps the curve of the shell smooth at 16px.
 *
 * Output:
 *   icons/tray/tray-template.png      16px, black + alpha (macOS template)
 *   icons/tray/tray-template@2x.png   32px
 *   icons/tray/tray-dark.png          16px, dark ink — for light taskbars
 *   icons/tray/tray-light.png         16px, light ink — for dark taskbars
 *   icons/tray/tray-dark@2x.png       32px
 *   icons/tray/tray-light@2x.png      32px
 */

import { deflateSync } from 'node:zlib'
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))
const OUT = join(HERE, '..', 'src-tauri', 'icons', 'tray')

/** Supersampling factor. 8 is plenty and keeps the script instant. */
const SS = 8

/**
 * The glyph, in a 16x16 unit square.
 *
 * Coordinates are chosen so that at 16px the flat underside of the shell and
 * the outer edge of each leg land on whole pixels — a half-pixel edge is what
 * makes a small icon look furry.
 */
const SHELL = { cx: 8, cy: 7.2, rx: 5.6, ry: 4.6 }
const BELLY = 1.4
const LEGS = [
  // [x1, y1, x2, y2] — mirrored about the centre line. Splayed wide rather
  // than hanging down: at this size a leg that reaches further sideways than
  // downwards is the one that still reads as a leg.
  [4.4, 7.4, 0.8, 8.8],
  [5.2, 8.6, 2.2, 11.2],
  [6.4, 9.0, 5.4, 12.6],
]
const LEG_WIDTH = 1.3

/** Distance from a point to a line segment, for stroking legs as capsules. */
function distanceToSegment(px, py, x1, y1, x2, y2) {
  const dx = x2 - x1
  const dy = y2 - y1
  const lengthSq = dx * dx + dy * dy
  const t = lengthSq === 0 ? 0 : Math.max(0, Math.min(1, ((px - x1) * dx + (py - y1) * dy) / lengthSq))
  const nx = x1 + t * dx
  const ny = y1 + t * dy
  return Math.hypot(px - nx, py - ny)
}

/** Whether a sample point at 16x16 scale is inside the creature. */
function covered(x, y) {
  // The shell: the top half of an ellipse, with a flat underside. Drawn last
  // in spirit — a leg that crosses it is simply part of the same silhouette.
  const ex = (x - SHELL.cx) / SHELL.rx
  const ey = (y - SHELL.cy) / SHELL.ry
  if (y <= SHELL.cy && ex * ex + ey * ey <= 1) return true
  // A shallow body below the dome, so the creature has a belly rather than
  // ending in a hard line.
  if (y > SHELL.cy && y <= SHELL.cy + BELLY && Math.abs(x - SHELL.cx) <= SHELL.rx * 0.99) return true

  for (const [x1, y1, x2, y2] of LEGS) {
    if (distanceToSegment(x, y, x1, y1, x2, y2) <= LEG_WIDTH / 2) return true
    // Mirrored.
    const m1 = 2 * SHELL.cx - x1
    const m2 = 2 * SHELL.cx - x2
    if (distanceToSegment(x, y, m1, y1, m2, y2) <= LEG_WIDTH / 2) return true
  }
  return false
}

/** Coverage per pixel, as a 0..1 alpha map. */
function rasterise(size) {
  const scale = 16 / size
  const alpha = new Float32Array(size * size)
  for (let py = 0; py < size; py++) {
    for (let px = 0; px < size; px++) {
      let hits = 0
      for (let sy = 0; sy < SS; sy++) {
        for (let sx = 0; sx < SS; sx++) {
          const x = (px + (sx + 0.5) / SS) * scale
          const y = (py + (sy + 0.5) / SS) * scale
          if (covered(x, y)) hits++
        }
      }
      alpha[py * size + px] = hits / (SS * SS)
    }
  }
  return alpha
}

function png(size, alpha, ink) {
  const raw = Buffer.alloc(size * (size * 4 + 1))
  let at = 0
  for (let y = 0; y < size; y++) {
    raw[at++] = 0 // filter: none
    for (let x = 0; x < size; x++) {
      const a = alpha[y * size + x]
      raw[at++] = ink[0]
      raw[at++] = ink[1]
      raw[at++] = ink[2]
      raw[at++] = Math.round(a * 255)
    }
  }

  const chunk = (type, data) => {
    const length = Buffer.alloc(4)
    length.writeUInt32BE(data.length)
    const body = Buffer.concat([Buffer.from(type, 'ascii'), data])
    const crc = Buffer.alloc(4)
    crc.writeUInt32BE(crc32(body) >>> 0)
    return Buffer.concat([length, body, crc])
  }

  const ihdr = Buffer.alloc(13)
  ihdr.writeUInt32BE(size, 0)
  ihdr.writeUInt32BE(size, 4)
  ihdr[8] = 8 // bit depth
  ihdr[9] = 6 // colour type: RGBA
  ihdr[10] = 0
  ihdr[11] = 0
  ihdr[12] = 0

  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ])
}

const CRC_TABLE = (() => {
  const table = new Int32Array(256)
  for (let n = 0; n < 256; n++) {
    let c = n
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1
    table[n] = c
  }
  return table
})()

function crc32(buf) {
  let c = 0xffffffff
  for (const byte of buf) c = CRC_TABLE[(c ^ byte) & 0xff] ^ (c >>> 8)
  return c ^ 0xffffffff
}

mkdirSync(OUT, { recursive: true })

// macOS template images are pure black plus alpha; the system recolours them
// for light and dark menu bars, and for when the menu is pulled down.
const INK = {
  template: [0, 0, 0],
  // Windows does not recolour tray icons, so both are supplied and the
  // taskbar's own theme picks one. Neither is pure black or pure white: an
  // icon that outshines everything beside it is its own kind of rude.
  dark: [0x2b, 0x27, 0x22],
  light: [0xef, 0xea, 0xe1],
}

for (const [size, suffix] of [
  [16, ''],
  [32, '@2x'],
]) {
  const alpha = rasterise(size)
  writeFileSync(join(OUT, `tray-template${suffix}.png`), png(size, alpha, INK.template))
  writeFileSync(join(OUT, `tray-dark${suffix}.png`), png(size, alpha, INK.dark))
  writeFileSync(join(OUT, `tray-light${suffix}.png`), png(size, alpha, INK.light))
}

console.log(`Wrote tray glyphs to ${OUT}`)
