#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""cursor_guest_pattern.py - GUEST side of scripts/bench/display/cursor_lane.sh: a full-screen window
on the guest's X display (Cinnamon/Xorg, DISPLAY=:0) with four corner markers the host grader finds
in the host screencast (to map host pixels to guest pixels), a flat grey field, and the cursor shape
named on the command line (an X cursor-theme name: left_ptr, xterm, crosshair, hand2 ...).

  cursor_guest_pattern.py [--cursor NAME] [--seconds S]

Markers (cursor_check.py MARKERS; change both together): 24x24, red (251,3,7) top-left, blue (3,7,251)
top-right, green (7,251,3) bottom-left, yellow (249,249,5) bottom-right (unusual values, so nothing else
on the host screen matches them exactly); field (100,100,100).
"""
import sys

import gi

gi.require_version("Gtk", "3.0")
gi.require_version("Gdk", "3.0")
from gi.repository import Gdk, GLib, Gtk  # noqa: E402

FIELD = (100, 100, 100)
MARKERS = {"tl": (251, 3, 7), "tr": (3, 7, 251), "bl": (7, 251, 3), "br": (249, 249, 5)}
M = 24


def main():
    name = sys.argv[sys.argv.index("--cursor") + 1] if "--cursor" in sys.argv else "left_ptr"
    seconds = int(sys.argv[sys.argv.index("--seconds") + 1]) if "--seconds" in sys.argv else 600
    w = Gtk.Window()
    w.set_decorated(False)
    area = Gtk.DrawingArea()
    w.add(area)

    def draw(widget, cr):
        a = widget.get_allocation()
        cr.set_source_rgb(*(v / 255 for v in FIELD))
        cr.paint()
        for key, (x, y) in {"tl": (0, 0), "tr": (a.width - M, 0), "bl": (0, a.height - M),
                            "br": (a.width - M, a.height - M)}.items():
            cr.set_source_rgb(*(v / 255 for v in MARKERS[key]))
            cr.rectangle(x, y, M, M)
            cr.fill()
        return False

    area.connect("draw", draw)

    def realize(*_):
        cur = Gdk.Cursor.new_from_name(w.get_display(), name)
        w.get_window().set_cursor(cur)
        print(f"PATTERN_UP cursor={name} themed={cur is not None}", flush=True)

    w.connect("realize", realize)
    w.connect("destroy", Gtk.main_quit)
    w.fullscreen()
    w.show_all()
    GLib.timeout_add_seconds(seconds, Gtk.main_quit)
    Gtk.main()


if __name__ == "__main__":
    main()
