import json
from gi.repository import Gio,GLib
try:
 bus=Gio.bus_get_sync(Gio.BusType.SESSION,None)
 r=bus.call_sync('org.gnome.Mutter.DisplayConfig','/org/gnome/Mutter/DisplayConfig','org.gnome.Mutter.DisplayConfig','GetCurrentState',None,None,Gio.DBusCallFlags.NONE,10000,None).unpack()
 print(json.dumps({'monitor_count':len(r[1]),'logical_monitor_count':len(r[2])}))
except Exception as e:
 print(json.dumps({'error':type(e).__name__}));raise SystemExit(1)
