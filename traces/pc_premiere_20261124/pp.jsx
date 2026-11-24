// Premiere Pro action A of PREREGISTRATION.md (after PR #588's pp_sed.jsx, `nosed` mode): new project, import one clip, sequence from the clip,
// make it active, select the clip, hold, write the done marker. Modes: "hold" (action A as on the Vast box), "probe" (log the QE renderer methods
// and the current renderer before the sequence step, then action A), "software" (switch the project renderer to Software Only through QE, read it
// back, then action A), "sed" (action A with Scene Edit Detection on the clip before the hold, the Vast 04:06 run), "learn" (CLIP is a
// project file: openDocument and return at once; pp_run.ps1 then watches the process for the hold). Every step is logged before it runs, so the last line before a crash names the step that was running.
var OUT = "@OUT@", CLIP = "@CLIP@", MODE = "@MODE@", HOLD = parseInt("@HOLD@", 10);
function L(m) { try { var f = new File(OUT + "/pp_progress.txt"); f.open("a"); f.writeln(new Date().getTime() + " " + m); f.close(); } catch (e) {} }
function count(seq) { var n = 0; for (var v = 0; v < seq.videoTracks.numTracks; v++) n += seq.videoTracks[v].clips.numItems; return n; }
function qeRenderer() {
  var r = [];
  try { r.push("current=" + qe.project.getCurrentRendererName()); } catch (e) { r.push("current_ERR=" + e); }
  try { var n = qe.project.getNumRenderers(); var names = []; for (var k = 0; k < n; k++) names.push(qe.project.getRendererName(k)); r.push("all=" + names.join("|")); } catch (e) { r.push("all_ERR=" + e); }
  return r.join(" ");
}
var res = [];
try {
  L("begin " + app.version + " mode=" + MODE + " clip=" + CLIP.replace(/^.*\//, ""));
  if (MODE == "learn") {
    // open and return at once: a script that keeps running (a $.sleep loop) keeps the project from loading
    L("creating sequence: openDocument");
    var ok = app.openDocument(CLIP, true, true, true);
    L("openDocument returned " + ok + "; sequence active and selected; returning so the project can load");
    res.push("learn opened=" + ok);
    throw new Error("learn: returned after openDocument");
  }
  app.newProject(OUT + "/pp.prproj");
  L("project created");
  if (MODE == "probe" || MODE == "software") {
    app.enableQE();
    var ms = []; try { var m = qe.project.reflect.methods; for (var i = 0; i < m.length; i++) if (/render/i.test(m[i].name)) ms.push(m[i].name); } catch (e) { ms.push("reflect_ERR=" + e); }
    L("qe.project renderer methods: " + ms.join(","));
    L("renderer before: " + qeRenderer());
    if (MODE == "software") {
      try {
        var n = qe.project.getNumRenderers(), sw = null;
        for (var k = 0; k < n; k++) { var nm = qe.project.getRendererName(k); if (/software/i.test(nm)) sw = nm; }
        L("software renderer name: " + sw);
        if (sw !== null) { var rr = qe.project.setRenderer(sw); L("setRenderer returned " + rr); }
      } catch (e) { L("setRenderer_ERR=" + String(e).replace(/[\r\n]+/g, " ")); }
      L("renderer after: " + qeRenderer());
      try { app.project.save(); L("project saved"); } catch (e) { L("save_ERR=" + e); }
    }
  }
  var bin = app.project.getInsertionBin();
  app.project.importFiles([CLIP], true, bin, false);
  var base = CLIP.replace(/^.*\//, "").replace(/\.[^.]*$/, "").toLowerCase();
  var item = null, t0 = new Date().getTime();
  while (!item && new Date().getTime() - t0 < 60000) {
    for (var i2 = 0; i2 < app.project.rootItem.children.numItems; i2++) { var c = app.project.rootItem.children[i2]; if (c.name.toLowerCase().indexOf(base) == 0) item = c; }
    if (!item) $.sleep(1000);
  }
  L("imported " + (item ? item.name : "none"));
  if (!item) throw new Error("clip not imported");
  L("creating sequence");
  var seq = app.project.createNewSequenceFromClips("ppseq", [item], bin);
  L("sequence created; setting active");
  app.project.activeSequence = seq;
  res.push("clips=" + count(seq));
  try { res.push("frame=" + seq.frameSizeHorizontal + "x" + seq.frameSizeVertical); } catch (e) {}
  var clip = seq.videoTracks[0].clips[0]; clip.setSelected(true, true);
  L("sequence active and selected; holding " + HOLD + " s");
  var t1 = new Date().getTime();
  if (MODE == "sed") {
    L("running scene edit detection");
    try { var r = seq.performSceneEditDetectionOnSelection("ApplyCuts", true, "MediumSensitivity"); res.push("sed_return=" + r); }
    catch (e) { res.push("sed_ERR=" + String(e).replace(/[\r\n]+/g, " ")); }
    res.push("sed_ms=" + (new Date().getTime() - t1)); res.push("clips_after=" + count(seq));
    L("scene edit detection returned; holding " + HOLD + " s"); t1 = new Date().getTime();
  }
  while (new Date().getTime() - t1 < HOLD * 1000) { $.sleep(5000); L("hold " + Math.round((new Date().getTime() - t1) / 1000) + " s"); }
  res.push("held=" + HOLD + "s");
  if (MODE == "probe" || MODE == "software") L("renderer at the end: " + qeRenderer());
} catch (e) { res.push("ERR=" + String(e).replace(/[\r\n]+/g, " ") + " line " + e.line); }
L("done " + res.join(" "));
var d = new File(OUT + "/pp_done.txt"); d.open("w"); d.writeln(res.join(" ")); d.close();
