// After Effects: build a project from nothing (no proprietary project file): a comp with a solid, optional footage, a text layer and
// GPU-capable effects, a render queue item writing a PNG sequence. Tokens are replaced by the PowerShell caller.
var OUT = "@OUT@", VIDEO = "@VIDEO@", MODE = "@MODE@", log = [];
try {
  app.beginSuppressDialogs();
  app.newProject();
  var comp = app.project.items.addComp("kfcomp", 1920, 1080, 1.0, 2, 30);
  var bg = comp.layers.addSolid([0.1, 0.2, 0.5], "bg", 1920, 1080, 1.0);
  var fx = bg.property("Effects");
  try { var fn = fx.addProperty("ADBE Fractal Noise"); log.push("fractalnoise"); } catch (e) { log.push("nofractal"); }
  try { fx.addProperty("ADBE Glo2"); log.push("glow"); } catch (e) { log.push("noglow"); }
  try { var gb = fx.addProperty("ADBE Gaussian Blur 2"); gb.property("Blurriness").setValue(12); log.push("blur"); } catch (e) { log.push("noblur"); }
  if (VIDEO != "") {
    try {
      var io = new ImportOptions(new File(VIDEO)); var foot = app.project.importFile(io);
      var vl = comp.layers.add(foot); vl.property("Effects").addProperty("ADBE Gaussian Blur 2").property("Blurriness").setValue(6);
      log.push("footage");
    } catch (e) { log.push("footagefail:" + String(e).substring(0, 60)); }
  }
  var tl = comp.layers.addText("kayfabe"); log.push("text");
  try { app.project.gpuAccelType = GpuAccelType.CUDA; log.push("cuda"); } catch (e) { log.push("gpuaccel_unavailable"); }
  var types = []; try { for (var k in app.availableGPUAccelTypes) types.push(String(app.availableGPUAccelTypes[k])); } catch (e) {}
  var rqi = app.project.renderQueue.items.add(comp);
  var om = rqi.outputModule(1);
  if (MODE == "ame") {
    var tl = ["H.264 - Match Render Settings - 15 Mbps", "H.264 - Match Render Settings - 40 Mbps"], tok = false;
    for (var ti = 0; ti < tl.length && !tok; ti++) { try { om.applyTemplate(tl[ti]); tok = true; log.push("om_h264"); } catch (e) { log.push("om_h264_missing"); } }
    om.file = new File(OUT + "/ame_out.mp4");
  } else {
    var tp = om.templates, pick = "", ext = "png";
    for (var k = 0; k < tp.length; k++) { if (/png/i.test(tp[k])) { pick = tp[k]; break; } }
    if (pick == "") { for (var k2 = 0; k2 < tp.length; k2++) { if (/^tiff sequence/i.test(tp[k2])) { pick = tp[k2]; ext = "tif"; break; } } }
    try { om.applyTemplate(pick); log.push("om:" + pick); } catch (e) { log.push("om_default:" + String(e).substring(0, 60)); }
    log.push("templates:" + tp.join("|"));
    om.file = new File(OUT + "/frames/f_[#####]." + ext);
  }
  app.project.save(new File(OUT + "/kf.aep"));
  if (MODE == "ame") { try { app.project.renderQueue.queueInAME(true); log.push("queued_in_ame"); } catch (e) { log.push("queue_fail:" + String(e).substring(0, 80)); } }
  var f = new File(OUT + "/ae_make.txt"); f.open("w"); f.writeln(log.join(",")); f.writeln("avail=" + types.join("|")); f.writeln("renderer=" + String(app.project.gpuAccelType)); f.close();
} catch (e) {
  var f2 = new File(OUT + "/ae_make.txt"); f2.open("w"); f2.writeln("ERR " + String(e) + " | " + log.join(",")); f2.close();
}
app.endSuppressDialogs(false);
if (MODE != "ame") { app.quit(); }
