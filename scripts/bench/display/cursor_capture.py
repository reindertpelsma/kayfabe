#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""cursor_capture.py - the HOST side of scripts/bench/display/cursor_lane.sh: what the host user SEES
(the broker's window with the host compositor's pointer or the broker's cursor overlay), and the host
pointer the user MOVES, through GNOME mutter's own D-Bus interfaces in the running session:

  org.gnome.Mutter.ScreenCast     RecordMonitor(connector, cursor-mode = 1 EMBEDDED): frames with the
                                  pointer drawn in, over PipeWire, read with GStreamer's pipewiresrc;
  org.gnome.Mutter.RemoteDesktop  NotifyPointerMotionAbsolute / ...Relative / NotifyKeyboardKeycode:
                                  the host pointer and keys, as a remote-desktop client injects them.

Run as the session's user (XDG_RUNTIME_DIR, DBUS_SESSION_BUS_ADDRESS of that session). Commands, one per
line on stdin; one reply line each on stdout:
  move X Y        absolute pointer, stream (monitor) pixels      -> CAP_OK move X Y
  rel DX DY       relative pointer motion                         -> CAP_OK rel DX DY
  key CODE 0|1    evdev keycode release/press                     -> CAP_OK key CODE S
  shot FILE.ppm   the newest frame (waits for one after the last command) -> CAP_OK shot FILE W H
  quit
Start-up prints CAP_READY connector=C w=W h=H, or CAP_ERR <why> and exits 3 (the route is unavailable:
not a GNOME session, the interfaces refused, no PipeWire node, no GStreamer pipewiresrc).
Hostile-guest note: nothing here reads guest memory; it reads the HOST's composited screen.
"""
import sys
import time

try:
    import gi

    gi.require_version("Gst", "1.0")
    from gi.repository import Gio, GLib, Gst
except (ImportError, ValueError) as e:  # pragma: no cover - environment
    print(f"CAP_ERR python gi/Gst unavailable: {e}", flush=True)
    sys.exit(3)

BUS = "org.gnome.Mutter.RemoteDesktop"
SC = "org.gnome.Mutter.ScreenCast"


def call(conn, name, path, iface, method, args=None, reply=None, timeout=10000):
    return conn.call_sync(name, path, iface, method, args, GLib.VariantType(reply) if reply else None,
                          Gio.DBusCallFlags.NONE, timeout, None)


def pump(ctx, seconds):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        ctx.iteration(False)
        time.sleep(0.01)


def main():
    connector = sys.argv[sys.argv.index("--connector") + 1] if "--connector" in sys.argv else ""
    Gst.init(None)
    conn = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    ctx = GLib.MainContext.default()
    try:
        rd = call(conn, BUS, "/org/gnome/Mutter/RemoteDesktop", BUS, "CreateSession", None, "(o)").unpack()[0]
        sid = conn.call_sync(BUS, rd, "org.freedesktop.DBus.Properties", "Get",
                             GLib.Variant("(ss)", (BUS + ".Session", "SessionId")), GLib.VariantType("(v)"),
                             Gio.DBusCallFlags.NONE, 5000, None).unpack()[0]
        sc = call(conn, SC, "/org/gnome/Mutter/ScreenCast", SC, "CreateSession",
                  GLib.Variant("(a{sv})", ({"remote-desktop-session-id": GLib.Variant("s", sid)},)), "(o)").unpack()[0]
        stream = call(conn, SC, sc, SC + ".Session", "RecordMonitor",
                      GLib.Variant("(sa{sv})", (connector, {"cursor-mode": GLib.Variant("u", 1)})), "(o)").unpack()[0]
    except GLib.Error as e:
        print(f"CAP_ERR mutter refused the remote-desktop/screencast session: {e.message}", flush=True)
        return 3
    node = []
    conn.signal_subscribe(SC, SC + ".Stream", "PipeWireStreamAdded", stream, None, Gio.DBusSignalFlags.NONE,
                          lambda *a: node.append(a[5].unpack()[0]))
    try:
        call(conn, BUS, rd, BUS + ".Session", "Start")
    except GLib.Error as e:
        print(f"CAP_ERR Start refused: {e.message}", flush=True)
        return 3
    for _ in range(500):
        if node:
            break
        pump(ctx, 0.02)
    if not node:
        print("CAP_ERR no PipeWire stream was announced", flush=True)
        return 3
    pipe = Gst.parse_launch(
        f"pipewiresrc path={node[0]} always-copy=true do-timestamp=true ! videoconvert ! "
        "video/x-raw,format=RGB ! appsink name=sink max-buffers=1 drop=true sync=false")
    sink = pipe.get_by_name("sink")
    pipe.set_state(Gst.State.PLAYING)

    def frame(min_after):
        """The newest frame whose arrival is after `min_after` (monotonic), as (w, h, bytes)."""
        end = time.monotonic() + 5
        got = None
        while time.monotonic() < end:
            pump(ctx, 0.01)
            s = sink.emit("try-pull-sample", 50 * Gst.MSECOND)
            if s is not None:
                got = (time.monotonic(), s)
                if got[0] >= min_after:
                    break
        if got is None:
            return None
        caps = got[1].get_caps().get_structure(0)
        w, h = caps.get_value("width"), caps.get_value("height")
        buf = got[1].get_buffer()
        ok, info = buf.map(Gst.MapFlags.READ)
        if not ok:
            return None
        stride = len(info.data) // h
        data = b"".join(bytes(info.data[y * stride:y * stride + w * 3]) for y in range(h))
        buf.unmap(info)
        return w, h, data

    first = frame(0)
    if first is None:
        print("CAP_ERR no frame from the screencast", flush=True)
        return 3
    print(f"CAP_READY connector={connector or 'primary'} w={first[0]} h={first[1]} node={node[0]}", flush=True)
    last_cmd = time.monotonic()
    for line in sys.stdin:
        p = line.split()
        if not p:
            continue
        try:
            if p[0] == "move":
                call(conn, BUS, rd, BUS + ".Session", "NotifyPointerMotionAbsolute",
                     GLib.Variant("(sdd)", (stream, float(p[1]), float(p[2]))))
            elif p[0] == "rel":
                call(conn, BUS, rd, BUS + ".Session", "NotifyPointerMotionRelative",
                     GLib.Variant("(dd)", (float(p[1]), float(p[2]))))
            elif p[0] == "key":
                call(conn, BUS, rd, BUS + ".Session", "NotifyKeyboardKeycode",
                     GLib.Variant("(ub)", (int(p[1]), p[2] == "1")))
            elif p[0] == "shot":
                time.sleep(0.3)
                f = frame(last_cmd + 0.25)
                if f is None:
                    print("CAP_ERR shot: no frame", flush=True)
                    continue
                with open(p[1], "wb") as out:
                    out.write(b"P6\n%d %d\n255\n" % (f[0], f[1]) + f[2])
                print(f"CAP_OK shot {p[1]} {f[0]} {f[1]}", flush=True)
                continue
            elif p[0] == "quit":
                break
            else:
                print(f"CAP_ERR unknown command {p[0]}", flush=True)
                continue
            last_cmd = time.monotonic()
            print("CAP_OK " + " ".join(p), flush=True)
        except (GLib.Error, IndexError, ValueError) as e:
            print(f"CAP_ERR {p[0]}: {e}", flush=True)
    pipe.set_state(Gst.State.NULL)
    try:
        call(conn, BUS, rd, BUS + ".Session", "Stop")
    except GLib.Error:
        pass
    return 0


if __name__ == "__main__":
    sys.exit(main())
