#!/usr/bin/env bash
# Records docs/media/demo.gif by driving the real Scuttle application.
#
# Nothing here is simulated. It builds a release app, points it at a pretend
# home directory full of invented files, records the window while a short
# scripted session runs, and converts the result. Every number and every
# filename you see in the finished GIF came out of the application reading
# those files.
#
#   scripts/demo/record.sh
#
# What it needs, once, on the Mac doing the recording:
#
#   * Screen Recording permission for the terminal application running this
#     (System Settings -> Privacy & Security -> Screen & System Audio Recording).
#   * Accessibility permission for the same application, so the script can
#     click Scuttle's buttons (System Settings -> Privacy & Security ->
#     Accessibility).
#   * ffmpeg:  brew install ffmpeg
#
# Both are prompted for the first time they are needed, and both require the
# application to be restarted afterwards.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
work="${TMPDIR:-/tmp}/scuttle-demo.$$"
home="$work/home"
media="$root/docs/media"
target="${SCUTTLE_DEMO_TARGET:-aarch64-apple-darwin}"
app="$root/src-tauri/target/$target/release/bundle/macos/Scuttle.app"

# The window is a fixed 1180x800 and cannot be resized, which is the one thing
# that makes an unattended recording reliable: the capture rectangle and every
# click coordinate below are stable across runs and machines.
WIDTH=1180
HEIGHT=800
ORIGIN_X=40
ORIGIN_Y=80

cleanup() {
  [[ -n "${recorder:-}" ]] && kill "$recorder" 2>/dev/null || true
  [[ -n "${appjob:-}" ]] && kill "$appjob" 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT

command -v ffmpeg >/dev/null || { echo 'ffmpeg is not installed: brew install ffmpeg'; exit 1; }

if ! screencapture -x "$work-probe.png" 2>/dev/null; then
  rm -f "$work-probe.png"
  cat >&2 <<'MSG'
Screen recording is not permitted for this application.

  System Settings -> Privacy & Security -> Screen & System Audio Recording
  enable the terminal (or editor) you are running this from, then restart it.

Nothing was recorded.
MSG
  exit 1
fi
rm -f "$work-probe.png"

# --- the pretend home ---------------------------------------------------------
mkdir -p "$work"
node "$root/scripts/demo/fixtures.mjs" "$home"

# --- the real application, built the way a release is -------------------------
if [[ ! -d "$app" ]]; then
  echo "Building Scuttle for $target (this takes a few minutes)..."
  # `app` as well as `dmg` so the bundle survives; asking for dmg alone cleans
  # the .app up once it has been packaged.
  ( cd "$root" && npm run tauri build -- --target "$target" --bundles app,dmg \
      --config src-tauri/tauri.apple-silicon.conf.json )
fi

# HOME is the whole isolation mechanism. Scuttle derives its scan roots, its
# database and its drawer from it, so with HOME pointed here the running
# application has no way to reach a real file: it has never been told where
# one is.
HOME="$home" "$app/Contents/MacOS/scuttle" &
appjob=$!
sleep 4

osascript <<APPLESCRIPT
tell application "System Events"
  tell process "Scuttle"
    set frontmost to true
    set position of window 1 to {$ORIGIN_X, $ORIGIN_Y}
  end tell
end tell
APPLESCRIPT
sleep 1

# --- record -------------------------------------------------------------------
# 20 fps is enough for an interface: the settling animation still reads, and
# the file stays small enough to sit in a README.
ffmpeg -hide_banner -loglevel error -y \
  -f avfoundation -capture_cursor 1 -framerate 20 -i "3:none" \
  -vf "crop=${WIDTH}:${HEIGHT}:${ORIGIN_X}:${ORIGIN_Y}" \
  -c:v libx264 -preset ultrafast -pix_fmt yuv420p \
  "$work/raw.mp4" &
recorder=$!
sleep 2

click() { # x y [pause]
  osascript -e "tell application \"System Events\" to tell process \"Scuttle\" to click at {$((ORIGIN_X + $1)), $((ORIGIN_Y + $2))}"
  sleep "${3:-1.5}"
}

# The session. Coordinates are relative to the window's own top-left corner.
# Keep the pauses generous: a demo that has to be paused to be read is a demo
# nobody reads.
source "$root/scripts/demo/session.sh"

sleep 2
kill "$recorder" 2>/dev/null || true
wait "$recorder" 2>/dev/null || true
kill "$appjob" 2>/dev/null || true

# --- convert ------------------------------------------------------------------
mkdir -p "$media"

# Two passes: one to work out a palette that suits this particular footage,
# one to apply it. A generic palette turns Scuttle's paper background into
# visible banding.
ffmpeg -hide_banner -loglevel error -y -i "$work/raw.mp4" \
  -vf "fps=14,scale=960:-1:flags=lanczos,palettegen=max_colors=96:stats_mode=diff" \
  "$work/palette.png"
ffmpeg -hide_banner -loglevel error -y -i "$work/raw.mp4" -i "$work/palette.png" \
  -lavfi "fps=14,scale=960:-1:flags=lanczos[v];[v][1:v]paletteuse=dither=bayer:bayer_scale=3:diff_mode=rectangle" \
  "$media/demo.gif"

# A still for anywhere the GIF does not play, and for the site's link preview.
# Taken from the findings screen rather than frame zero, so the fallback shows
# the product rather than an empty window.
ffmpeg -hide_banner -loglevel error -y -i "$work/raw.mp4" \
  -vf "select=eq(n\,${SCUTTLE_DEMO_POSTER_FRAME:-220}),scale=1180:-1" -vframes 1 \
  "$media/findings.png"

cp "$work/raw.mp4" "$media/demo.mp4"

echo
echo "Wrote:"
ls -lh "$media/demo.gif" "$media/demo.mp4" "$media/findings.png"
echo
echo "If demo.gif is over about 6 MB, lower the fps or the scale above."
