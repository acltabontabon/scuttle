#!/usr/bin/env bash
# Records docs/media/demo.gif, demo.mp4 and findings.png by driving the real
# Scuttle application.
#
# Nothing here is simulated. It builds a release app, points it at a pretend
# home directory full of invented files, films the window while a short
# scripted session runs, and converts the result. Every number and every
# filename in the finished recording came out of the application reading
# those files.
#
#   scripts/demo/record.sh
#
# What it needs, once, on the Mac doing the recording — both under System
# Settings -> Privacy & Security, both prompted the first time they are
# needed, and both applying to whatever terminal or editor is running this:
#
#   * Screen & System Audio Recording, to film the window.
#   * Accessibility, to click the application's buttons. macOS caches this
#     per process, so the application running the script has to be restarted
#     after granting it.
#
# And ffmpeg:  brew install ffmpeg

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
work="$(mktemp -d "${TMPDIR:-/tmp}/scuttle-demo.XXXXXX")"
home="$work/home"
media="$root/docs/media"
target="${SCUTTLE_DEMO_TARGET:-aarch64-apple-darwin}"
app="$root/src-tauri/target/$target/release/bundle/macos/Scuttle.app"

# The window is a fixed 1180x800 and cannot be resized, which is what makes
# an unattended recording reproducible: one capture rectangle, every time.
WIN_W=1180
WIN_H=800
WIN_X=120
WIN_Y=120
# A macOS window is a rounded rectangle, so a crop of its exact bounds picks
# up four small triangles of whatever happened to be behind it. Rather than
# cropping inside — which clips the traffic lights — the corners are filled
# with the window background colour afterwards, leaving a clean rectangle
# with the whole title bar in it. This is the colour tauri.conf.json gives
# the window, so the patch is invisible.
CORNER=16
PAPER=0xF4EFE6

appjob=""
recorder=""
cleanup() {
  [[ -n "$recorder" ]] && kill "$recorder" 2>/dev/null || true
  [[ -n "$appjob" ]] && kill "$appjob" 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT

command -v ffmpeg > /dev/null || { echo 'ffmpeg is not installed: brew install ffmpeg' >&2; exit 1; }

if ! screencapture -x "$work/probe.png" 2> /dev/null; then
  cat >&2 <<'MSG'
Screen recording is not permitted for this application.

  System Settings -> Privacy & Security -> Screen & System Audio Recording
  enable the terminal (or editor) running this, then try again.

Nothing was recorded.
MSG
  exit 1
fi

if ! osascript -e 'tell application "System Events" to return name of first process' > /dev/null 2>&1; then
  cat >&2 <<'MSG'
Accessibility is not permitted for this application, so the session cannot
click anything.

  System Settings -> Privacy & Security -> Accessibility
  enable the terminal (or editor) running this, then RESTART it: macOS caches
  this permission per process and will not notice otherwise.

Nothing was recorded.
MSG
  exit 1
fi

# --- the pretend home -------------------------------------------------------
# HOME is the whole isolation mechanism. Scuttle derives its scan roots, its
# database and its drawer from it, so with HOME pointed here the running
# application has no way to reach a real file: it has never been told where
# one is.
node "$here/fixtures.mjs" "$home"

# --- the real application, built the way a release is -----------------------
if [[ ! -d "$app" ]]; then
  echo "Building Scuttle for $target (a few minutes)…"
  ( cd "$root" && npm run tauri build -- \
      --target "$target" --bundles app \
      --config src-tauri/tauri.apple-silicon.conf.json )
fi

pkill -f 'Scuttle.app/Contents/MacOS/scuttle' 2> /dev/null || true
sleep 1
HOME="$home" "$app/Contents/MacOS/scuttle" > "$work/app.log" 2>&1 &
appjob=$!

# Wait for the interface to be there rather than guessing at a sleep: the
# window appears before the webview has published its accessibility tree, and
# clicking into the gap fails with a confusing "invalid index".
ready=no
for _ in $(seq 1 45); do
  if "$here/ui.sh" list 2> /dev/null | grep -q Rummage; then ready=yes; break; fi
  sleep 1
done
if [[ "$ready" != yes ]]; then
  echo '::error::Scuttle never showed a Rummage button. Its log:' >&2
  tail -20 "$work/app.log" >&2
  exit 1
fi

osascript > /dev/null <<APPLESCRIPT
tell application "System Events" to tell process "Scuttle"
  set frontmost to true
  set position of window 1 to {$WIN_X, $WIN_Y}
end tell
APPLESCRIPT
sleep 1

# --- film it ----------------------------------------------------------------
# The screen is captured in physical pixels and the window is placed in
# points, so every coordinate doubles on a Retina display.
scale=2
crop_w=$(( WIN_W * scale ))
crop_h=$(( WIN_H * scale ))
crop_x=$(( WIN_X * scale ))
crop_y=$(( WIN_Y * scale ))
c=$(( CORNER * scale ))
corners="drawbox=0:0:${c}:${c}:color=${PAPER}:t=fill"
corners="${corners},drawbox=$(( crop_w - c )):0:${c}:${c}:color=${PAPER}:t=fill"
corners="${corners},drawbox=0:$(( crop_h - c )):${c}:${c}:color=${PAPER}:t=fill"
corners="${corners},drawbox=$(( crop_w - c )):$(( crop_h - c )):${c}:${c}:color=${PAPER}:t=fill"

# `-list_devices` prints the list and then exits non-zero, every time, which
# under `pipefail` would take the whole script with it.
screen=$(ffmpeg -hide_banner -f avfoundation -list_devices true -i "" 2>&1 \
  | sed -n 's/.*\[\([0-9]*\)\] Capture screen 0.*/\1/p' | head -1 || true)
: "${screen:=3}"
echo "Filming screen device $screen, window ${WIN_W}x${WIN_H} at ${WIN_X},${WIN_Y}."

# `-pixel_format` belongs to the input: the screen device offers uyvy422 and
# friends, not the yuv420p the encoder wants, and asking it for yuv420p is
# refused rather than converted.
ffmpeg -hide_banner -loglevel error -y \
  -f avfoundation -capture_cursor 1 -framerate 30 -pixel_format uyvy422 -i "${screen}:none" \
  -vf "crop=${crop_w}:${crop_h}:${crop_x}:${crop_y},${corners}" \
  -c:v libx264 -preset ultrafast -qp 0 -pix_fmt yuv420p \
  "$work/raw.mp4" &
recorder=$!
sleep 2.5

ui() { "$here/ui.sh" "$@" > /dev/null; }
beat() { sleep "$1"; }

# shellcheck source=session.sh
source "$here/session.sh"

sleep 1.5
kill "$recorder" 2> /dev/null || true
wait "$recorder" 2> /dev/null || true
recorder=""
kill "$appjob" 2> /dev/null || true
appjob=""

# --- convert ----------------------------------------------------------------
mkdir -p "$media"

# A small h.264 copy for the site, which offers it behind a button rather
# than making everybody download it to read a paragraph.
ffmpeg -hide_banner -loglevel error -y -i "$work/raw.mp4" \
  -vf "scale=1180:-2:flags=lanczos" \
  -c:v libx264 -preset slow -crf 26 -pix_fmt yuv420p -movflags +faststart -an \
  "$media/demo.mp4"

# Two passes for the GIF: one to work out a palette that suits this footage,
# one to apply it. A generic palette turns Scuttle's paper into banding.
ffmpeg -hide_banner -loglevel error -y -i "$work/raw.mp4" \
  -vf "fps=12,scale=900:-1:flags=lanczos,palettegen=max_colors=96:stats_mode=diff" \
  "$work/palette.png"
ffmpeg -hide_banner -loglevel error -y -i "$work/raw.mp4" -i "$work/palette.png" \
  -lavfi "fps=12,scale=900:-1:flags=lanczos[v];[v][1:v]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle" \
  "$media/demo.gif"

# A still for anywhere the recording does not play, and for the link preview.
# Taken from the findings screen rather than frame zero, so what somebody sees
# first is the product rather than an empty window.
ffmpeg -hide_banner -loglevel error -y -ss "${SCUTTLE_DEMO_POSTER_AT:-9}" -i "$work/raw.mp4" \
  -vf "scale=2360:-2" -frames:v 1 "$media/findings.png"

# And one of the drawer holding something, for the section of the site that
# is about the drawer. Both are frames of this recording rather than separate
# captures, so they cannot drift from it.
ffmpeg -hide_banner -loglevel error -y -ss "${SCUTTLE_DEMO_DRAWER_AT:-22}" -i "$work/raw.mp4" \
  -vf "scale=2360:-2" -frames:v 1 "$media/drawer.png"

echo
ls -lh "$media"/demo.gif "$media"/demo.mp4 "$media"/findings.png "$media"/drawer.png
echo
echo "If demo.gif is much over 6 MB, drop the fps or the width in the two passes above."
