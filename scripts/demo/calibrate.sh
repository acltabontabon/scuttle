#!/usr/bin/env bash
# Prints the centre of everything clickable on whatever screen Scuttle is
# currently showing, in window coordinates, ready to paste into coords.sh.
#
# Run it with Scuttle already open and sitting on the screen you want to
# measure, then move it to the next screen and run it again:
#
#   scripts/demo/calibrate.sh
#
# Needs Accessibility permission for the terminal running it (System Settings
# -> Privacy & Security -> Accessibility). Reading the positions out of the
# running application beats measuring a screenshot, because it keeps working
# when the layout changes.

set -euo pipefail

osascript <<'APPLESCRIPT'
tell application "System Events"
  if not (exists process "scuttle") then
    return "Scuttle is not running. Start it first — scripts/demo/record.sh does this for you."
  end if
  tell process "scuttle"
    set w to window 1
    set {wx, wy} to position of w
    set report to "window at " & wx & "," & wy & return
    -- The webview presents its controls as a flat-ish tree; entire contents
    -- walks all of it, which is what we want rather than one level.
    repeat with e in (entire contents of w)
      try
        set r to role of e
        if r is in {"AXButton", "AXCheckBox", "AXLink", "AXRadioButton"} then
          set {ex, ey} to position of e
          set {ew, eh} to size of e
          set label to ""
          try
            set label to name of e
          end try
          if label is "" then
            try
              set label to description of e
            end try
          end if
          set report to report & "  " & (ex - wx + (ew div 2)) & " " & ¬
            (ey - wy + (eh div 2)) & "   " & r & "  " & label & return
        end if
      end try
    end repeat
    return report
  end tell
end tell
APPLESCRIPT
