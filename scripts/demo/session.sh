# The scripted session that record.sh films. Sourced, not run on its own: it
# uses the `click` helper record.sh defines and the coordinates in coords.sh.
#
# The story, in order: rummage, watch the piles settle, open the pile of
# leftovers, tick one, put it in the drawer, go to the drawer, put it back.
# It deliberately ends where it started — "you can always undo this" is the
# most important thing about Scuttle, and a demo that ends with something
# deleted says the opposite.
#
# Pauses are generous on purpose. A demo that has to be paused to be read is a
# demo nobody reads.

# shellcheck source=coords.sh
source "$root/scripts/demo/coords.sh"

# --- Find it ------------------------------------------------------------------
sleep 2.5                       # the home screen: the creature, one button
click $AT_RUMMAGE 4             # the scan narrates what it is looking through
sleep 3                         # and hands over to the findings on its own

# --- The floor ----------------------------------------------------------------
sleep 3.5                       # the piles settle; let them finish

# --- Review it ----------------------------------------------------------------
click $AT_GHOSTS_PILE 3         # into the pile of leftovers
click $AT_FIRST_ITEM 1.5        # tick one, and the meter starts counting
sleep 1.5

# --- Put it in the drawer -----------------------------------------------------
click $AT_SWEEP 3               # nothing is deleted; it is just out of the way
sleep 1.5

# --- And back out again -------------------------------------------------------
click $AT_DRAWER_TAB 3
click $AT_PUT_IT_BACK 3.5       # back exactly where it came from
sleep 2.5
