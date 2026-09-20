# The session record.sh films. Sourced, not run on its own: `ui` and `beat`
# come from there.
#
# The story, in order: rummage, watch the piles settle, open the pile of
# leftovers, tick one, put it in the drawer, go and look at the drawer, put it
# back. It deliberately ends where it started — "you can always undo this" is
# the most important thing about Scuttle, and a demo that ends with something
# deleted says the opposite.
#
# Pauses are generous enough to read and no longer. A demo that has to be
# paused to be followed is a demo nobody follows; one that idles is a demo
# nobody finishes.

beat 2.0                            # the home screen: a creature and a button

ui click "Rummage"
beat 3.8                            # the scan says what it is looking through,
                                    # then hands over to the findings itself
beat 3.0                            # the piles settle onto their floor

ui click "Ghosts —"                 # into the pile of leftovers
beat 2.6                            # two of them, each with its evidence

ui click "Select com.harborlight"   # tick one, and the meter starts counting
beat 1.5

ui click "Put it in the drawer"     # nothing is deleted; it is out of the way
beat 2.4

ui click "Drawer 1"                 # where it went, and where it came from
beat 2.8

ui click "Put it back"              # and back to exactly where it was
beat 2.4
