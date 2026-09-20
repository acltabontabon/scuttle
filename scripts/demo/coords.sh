# Where to click, in Scuttle's own window coordinates.
#
# Stable because the window is a fixed 1180x800 and cannot be resized. Each
# value is "x y", the centre of the thing being clicked.
#
# Regenerate after an interface change with scripts/demo/calibrate.sh, which
# reads the positions out of the running application rather than guessing.

AT_RUMMAGE=""        # home: the Rummage button
AT_GHOSTS_PILE=""    # the floor: the pile of leftovers from uninstalled software
AT_FIRST_ITEM=""     # inside the pile: the first item's checkbox
AT_SWEEP=""          # inside the pile: "Put it in the drawer"
AT_DRAWER_TAB=""     # the nav: Drawer
AT_PUT_IT_BACK=""    # the drawer: "Put it back" on the first card
