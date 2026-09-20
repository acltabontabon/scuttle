#!/usr/bin/env bash
# Drives Scuttle's interface by name rather than by coordinate.
#
#   scripts/demo/ui.sh list              every clickable thing on this screen
#   scripts/demo/ui.sh click "<text>"    click the first whose label contains it
#
# The webview publishes a real accessibility tree — the same one a screen
# reader uses — so the recording can ask for "Put it in the drawer" instead of
# a pair of pixel coordinates that stop being true the next time anything
# moves. It also means a session that no longer works fails loudly, naming
# what it could not find, rather than quietly clicking empty paper.
#
# Needs Accessibility permission for whatever is running it (System Settings
# -> Privacy & Security -> Accessibility).

set -euo pipefail

action="${1:-list}"
wanted="${2:-}"

result=$(osascript <<APPLESCRIPT
tell application "System Events"
  if not (exists process "Scuttle") then return "ERROR: Scuttle is not running"
  tell process "Scuttle"
    set frontmost to true
    delay 0.25
    set {wx, wy} to position of window 1
    -- Native window chrome wraps the webview, and how deeply depends on when
    -- you ask: the tree is not published until the page has loaded. Try the
    -- usual shape first and fall back to the whole window.
    set elems to {}
    try
      set elems to entire contents of scroll area 1 of group 1 of group 1 of window 1
    on error
      try
        set elems to entire contents of group 1 of window 1
      on error
        set elems to entire contents of window 1
      end try
    end try
    set out to ""
    repeat with e in elems
      try
        set r to role of e
        if r is in {"AXButton", "AXCheckBox", "AXLink"} then
          set lbl to ""
          try
            set lbl to name of e
          end try
          if "$action" is "list" then
            set {ex, ey} to position of e
            set {ew, eh} to size of e
            set out to out & (ex - wx + (ew div 2)) & "," & (ey - wy + (eh div 2)) & "  " & r & "  [" & lbl & "]" & linefeed
          else if lbl contains "$wanted" then
            set {ex, ey} to position of e
            set {ew, eh} to size of e
            click at {ex + (ew div 2), ey + (eh div 2)}
            return "OK " & lbl
          end if
        end if
      end try
    end repeat
    if "$action" is "list" then return out
    return "ERROR: nothing on this screen is labelled: $wanted"
  end tell
end tell
APPLESCRIPT
)

printf '%s\n' "$result"
case "$result" in
  ERROR*) exit 1 ;;
esac
