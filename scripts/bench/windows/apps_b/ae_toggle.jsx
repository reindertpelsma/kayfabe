// After Effects exotic row: the renderer switch. One comp is rendered as a single frame three times: CUDA, SOFTWARE, CUDA again, switching
// app.project.gpuAccelType between the renders inside one session (a path vendors document as unsupported to do mid-session). Tokens are
// replaced by the PowerShell caller. Every step is logged; the host compares the three frames.
var OUT = "@OUT@", log = [];
function render(tag, type) {
  var ok = false;
  try { app.project.gpuAccelType = type; log.push(tag + ":set=" + String(app.project.gpuAccelType)); } catch (e) { log.push(tag + ":set_fail:" + String(e).substring(0, 60)); }
  try {
    var comp = app.project.item(1);
    var rq = app.project.renderQueue;
    while (rq.numItems > 0) { rq.item(1).remove(); }
    var rqi = rq.items.add(comp);
    var om = rqi.outputModule(1);
    var tp = om.templates, pick = "", ext = "png";
    for (var k = 0; k < tp.length; k++) { if (/png/i.test(tp[k])) { pick = tp[k]; break; } }
    if (pick == "") { for (var k2 = 0; k2 < tp.length; k2++) { if (/^tiff sequence/i.test(tp[k2])) { pick = tp[k2]; ext = "tif"; break; } } }
    if (pick != "") { om.applyTemplate(pick); }
    om.file = new File(OUT + "/toggle_" + tag + "_[#####]." + ext);
    rqi.timeSpanStart = 0.5; rqi.timeSpanDuration = 1 / 30;
    var t0 = new Date().getTime();
    rq.render();
    log.push(tag + ":rendered_ms=" + (new Date().getTime() - t0));
    ok = true;
  } catch (e) { log.push(tag + ":render_fail:" + String(e).substring(0, 80)); }
  return ok;
}
try {
  app.beginSuppressDialogs();
  app.newProject();
  var W = 3840, H = 2160;
  var comp = app.project.items.addComp("kftoggle", W, H, 1.0, 2, 30);
  var bg = comp.layers.addSolid([0.1, 0.2, 0.5], "bg", W, H, 1.0);
  var fx = bg.property("Effects");
  try { fx.addProperty("ADBE Fractal Noise").property("Complexity").setValue(8); log.push("fractalnoise"); } catch (e) { log.push("no_fractalnoise"); }
  try { fx.addProperty("ADBE Glo2"); log.push("glow"); } catch (e) { log.push("no_glow"); }
  try { fx.addProperty("ADBE Gaussian Blur 2").property("Blurriness").setValue(20); log.push("blur"); } catch (e) { log.push("no_blur"); }
  var types = []; try { for (var k in app.availableGPUAccelTypes) types.push(String(app.availableGPUAccelTypes[k])); } catch (e) {}
  log.push("avail=" + types.join("|"));
  render("cuda1", GpuAccelType.CUDA);
  render("soft", GpuAccelType.SOFTWARE);
  render("cuda2", GpuAccelType.CUDA);
} catch (e) { log.push("ERR " + String(e).substring(0, 100)); }
app.endSuppressDialogs(false);
var f = new File(OUT + "/ae_toggle.txt"); f.open("w"); f.writeln(log.join(",")); f.close();
app.quit();
