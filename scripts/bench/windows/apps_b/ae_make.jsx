// After Effects: build a feature-dense project from nothing (no proprietary project file): a 4K 16-bit comp with 3D layers, a nested
// precomp, expressions, GPU-capable effects (fractal noise, glow, blur, turbulent displace, particles), optional footage and text, and a
// render queue item. Tokens are replaced by the PowerShell caller. Every feature is recorded; the host decides what is required.
var OUT = "@OUT@", VIDEO = "@VIDEO@", MODE = "@MODE@", log = [];
function tryFx(layer, match, name, setup) {
  try { var fx = layer.property("Effects").addProperty(match); if (setup) setup(fx); log.push(name); return fx; } catch (e) { log.push("no_" + name); return null; }
}
try {
  app.beginSuppressDialogs();
  app.newProject();
  try { app.project.bitsPerChannel = 16; log.push("16bpc"); } catch (e) { log.push("no_16bpc"); }
  var W = (MODE == "ame") ? 1920 : 3840, H = (MODE == "ame") ? 1080 : 2160;
  var comp = app.project.items.addComp("kfcomp", W, H, 1.0, 2, 30);
  var inner = app.project.items.addComp("kfinner", W, H, 1.0, 2, 30);
  var bg = inner.layers.addSolid([0.1, 0.2, 0.5], "bg", W, H, 1.0);
  tryFx(bg, "ADBE Fractal Noise", "fractalnoise", function (fx) { fx.property("Complexity").setValue(8); });
  tryFx(bg, "ADBE Turbulent Displace", "turbulent");
  tryFx(bg, "ADBE Glo2", "glow");
  tryFx(bg, "ADBE Gaussian Blur 2", "blur", function (fx) { fx.property("Blurriness").setValue(12); });
  var pre = comp.layers.add(inner); log.push("precomp");
  if (VIDEO != "") {
    try {
      var foot = app.project.importFile(new ImportOptions(new File(VIDEO)));
      var vl = comp.layers.add(foot);
      var gb = vl.property("Effects").addProperty("ADBE Gaussian Blur 2"); gb.property("Blurriness").setValue(6);
      tryFx(vl, "ADBE Lumetri", "lumetri");
      try { vl.threeDLayer = true; vl.property("Position").expression = "[value[0]+wiggle(2,60)[0], value[1], value[2]+100*Math.sin(time*3)]"; log.push("3d_layer"); log.push("expression"); } catch (e) { log.push("no_3d_expr"); }
      log.push("footage");
    } catch (e) { log.push("footagefail:" + String(e).substring(0, 60)); }
  }
  var cam = null;
  try { cam = comp.layers.addCamera("kfcam", [W / 2, H / 2]); log.push("camera"); } catch (e) {}
  try {
    var sl = comp.layers.addSolid([0.9, 0.4, 0.1], "particles", W, H, 1.0);
    tryFx(sl, "CC Particle World", "particles");
    sl.threeDLayer = true; sl.blendingMode = BlendingMode.ADD;
  } catch (e) { log.push("no_particles"); }
  try { var tl = comp.layers.addText("kayfabe"); tl.threeDLayer = true; tl.property("Position").expression = "wiggle(3,40)"; log.push("text"); } catch (e) { log.push("no_text"); }
  try { app.project.gpuAccelType = GpuAccelType.CUDA; log.push("cuda"); } catch (e) { log.push("gpuaccel_unavailable"); }
  var types = []; try { for (var k in app.availableGPUAccelTypes) types.push(String(app.availableGPUAccelTypes[k])); } catch (e) {}
  var rqi = app.project.renderQueue.items.add(comp);
  var om = rqi.outputModule(1);
  if (MODE == "ame") {
    var tlist = ["H.264 - Match Render Settings - 15 Mbps", "H.264 - Match Render Settings - 40 Mbps"], tok = false;
    for (var ti = 0; ti < tlist.length && !tok; ti++) { try { om.applyTemplate(tlist[ti]); tok = true; log.push("om_h264"); } catch (e) { log.push("om_h264_missing"); } }
    om.file = new File(OUT + "/ame_out.mp4");
  } else {
    var tp = om.templates, pick = "", ext = "png";
    for (var k1 = 0; k1 < tp.length; k1++) { if (/png/i.test(tp[k1])) { pick = tp[k1]; break; } }
    if (pick == "") { for (var k2 = 0; k2 < tp.length; k2++) { if (/^tiff sequence/i.test(tp[k2])) { pick = tp[k2]; ext = "tif"; break; } } }
    try { om.applyTemplate(pick); log.push("om:" + pick); } catch (e) { log.push("om_default:" + String(e).substring(0, 60)); }
    log.push("templates:" + tp.join("|"));
    om.file = new File(OUT + "/frames/f_[#####]." + ext);
  }
  app.project.save(new File(OUT + "/kf.aep"));
  if (MODE == "ame") { try { app.project.renderQueue.queueInAME(true); log.push("queued_in_ame"); } catch (e) { log.push("queue_fail:" + String(e).substring(0, 80)); } }
  var f = new File(OUT + "/ae_make.txt"); f.open("w"); f.writeln(log.join(",")); f.writeln("avail=" + types.join("|")); f.writeln("renderer=" + String(app.project.gpuAccelType)); f.writeln("size=" + W + "x" + H); f.close();
} catch (e) {
  var f2 = new File(OUT + "/ae_make.txt"); f2.open("w"); f2.writeln("ERR " + String(e) + " | " + log.join(",")); f2.close();
}
app.endSuppressDialogs(false);
if (MODE != "ame") { app.quit(); }
