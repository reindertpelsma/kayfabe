// After Effects exotic row: the Advanced 3D renderer with depth of field, many 3D layers at different depths, a light, 16 bit, 4K.
var OUT = "@OUT@", log = [];
function P(m) { try { var pf = new File(OUT + "/ae_progress.txt"); pf.open("a"); pf.writeln(new Date().getTime() + " " + m); pf.close(); } catch (e) {} }
P("script start");
try {
  app.beginSuppressDialogs();
  app.newProject();
  try { app.project.bitsPerChannel = 16; log.push("16bpc"); } catch (e) { log.push("no_16bpc"); }
  var W = 3840, H = 2160;
  var comp = app.project.items.addComp("kfcomp", W, H, 1.0, 2, 30);
  try { var rs = comp.renderers; log.push("renderers=" + rs.join("|")); comp.renderer = "ADBE Advanced 3d"; log.push("renderer=" + comp.renderer); } catch (e) { log.push("renderer_fail:" + String(e).substring(0, 80)); }
  var cam = comp.layers.addCamera("cam", [W / 2, H / 2]);
  try { var co = cam.property("Camera Options"); co.property("Depth of Field").setValue(1); co.property("Aperture").setValue(120); co.property("Focus Distance").setValue(1400); log.push("dof"); } catch (e) { log.push("no_dof:" + String(e).substring(0, 60)); }
  try { var lt = comp.layers.addLight("light", [W / 2, H / 3]); lt.property("Transform").property("Position").setValue([W / 2, H / 4, -800]); log.push("light"); } catch (e) { log.push("no_light"); }
  var n3 = 0;
  for (var i = 0; i < 24; i++) {
    var c = [0.2 + (i % 5) * 0.15, 0.8 - (i % 7) * 0.1, 0.3 + (i % 3) * 0.2];
    var sl = comp.layers.addSolid(c, "s" + i, 1200, 800, 1.0);
    sl.threeDLayer = true;
    var t = sl.property("Transform");
    t.property("Position").setValue([300 + (i * 337) % (W - 600), 300 + (i * 211) % (H - 600), (i - 12) * 220]);
    t.property("Y Rotation").setValue((i * 17) % 60 - 30);
    try { sl.property("Material Options").property("Casts Shadows").setValue(1); } catch (e) {}
    try { sl.property("Effects").addProperty("ADBE Gaussian Blur 2").property("Blurriness").setValue(3); } catch (e) {}
    n3++;
  }
  log.push("layers3d=" + n3);
  var rqi = app.project.renderQueue.items.add(comp);
  var om = rqi.outputModule(1);
  var tp = om.templates, pick = "", ext = "png";
  for (var k = 0; k < tp.length; k++) { if (/png/i.test(tp[k])) { pick = tp[k]; break; } }
  if (pick == "") { for (var k2 = 0; k2 < tp.length; k2++) { if (/^tiff sequence/i.test(tp[k2])) { pick = tp[k2]; ext = "tif"; break; } } }
  try { om.applyTemplate(pick); log.push("om:" + pick); } catch (e) { log.push("om_default"); }
  om.file = new File(OUT + "/frames/f_[#####]." + ext);
  rqi.timeSpanStart = 0; rqi.timeSpanDuration = 10 / 30;
  app.project.save(new File(OUT + "/kf.aep"));
  P("saved");
} catch (e) { log.push("ERR " + String(e).substring(0, 100)); }
app.endSuppressDialogs(false);
var f = new File(OUT + "/ae_make.txt"); f.open("w"); f.writeln(log.join(",")); f.close();
app.quit();
