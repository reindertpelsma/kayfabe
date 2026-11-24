// Premiere Pro (ExtendScript through the Startup stub): new project, import the data-volume trailer, sequence from the clip, then Scene Edit
// Detection on the selected clip (Sequence.performSceneEditDetectionOnSelection). Records clip counts before and after and any error verbatim.
var OUT = "@OUT@", CLIP = "@CLIP@", RENDERER = "@RENDERER@";
function L(m) { try { var f = new File(OUT + "/pp_progress.txt"); f.open("a"); f.writeln(new Date().getTime() + " " + m); f.close(); } catch (e) {} }
function count(seq) { var n = 0; for (var v = 0; v < seq.videoTracks.numTracks; v++) n += seq.videoTracks[v].clips.numItems; return n; }
var res = [];
try {
  L("begin " + app.version);
  app.newProject(OUT + "/sed.prproj");
  var bin = app.project.getInsertionBin();
  app.project.importFiles([CLIP], true, bin, false);
  var item = null, t0 = new Date().getTime();
  while (!item && new Date().getTime() - t0 < 60000) { for (var i = 0; i < app.project.rootItem.children.numItems; i++) { var c = app.project.rootItem.children[i]; if (/sintel/i.test(c.name)) item = c; } if (!item) $.sleep(1000); }
  L("imported " + (item ? item.name : "none"));
  var seq = app.project.createNewSequenceFromClips("sed", [item], bin);
  app.project.activeSequence = seq;
  res.push("clips_before=" + count(seq));
  if (RENDERER == "software") {
    try { app.enableQE(); var rs = qe.project.getSupportedRendererNames ? "" : ""; } catch (e) {}
    try { var ids = app.project.getSupportedRenderers ? app.project.getSupportedRenderers() : null; res.push("renderers=" + ids); } catch (e) { res.push("renderers_ERR=" + e); }
    try { app.enableQE(); var n = qe.project.getNumRenderers(); var names = []; for (var k = 0; k < n; k++) names.push(qe.project.getRendererName(k)); res.push("qe_renderers=" + names.join("|"));
          var sw = -1; for (var k2 = 0; k2 < n; k2++) if (/Software/i.test(names[k2])) sw = k2;
          if (sw >= 0) { qe.project.setRenderer(qe.project.getRendererName(sw)); } res.push("qe_current=" + qe.project.getCurrentRendererName()); }
    catch (e) { res.push("qe_ERR=" + String(e).replace(/[\r\n]+/g, " ")); }
  }
  var clip = seq.videoTracks[0].clips[0]; clip.setSelected(true, true);
  L("selected; running scene edit detection");
  var t1 = new Date().getTime(), r;
  if (RENDERER == "nosed") { L("no scene edit detection: holding the sequence open 45 s"); $.sleep(45000); res.push("held=45s"); }
  else try { r = seq.performSceneEditDetectionOnSelection("ApplyCuts", true, "MediumSensitivity"); res.push("sed_return=" + r); }
  catch (e) { res.push("sed_ERR=" + String(e).replace(/[\r\n]+/g, " ")); }
  res.push("sed_ms=" + (new Date().getTime() - t1));
  res.push("clips_after=" + count(seq));
} catch (e) { res.push("ERR=" + String(e).replace(/[\r\n]+/g, " ") + " line " + e.line); }
L("done " + res.join(" "));
var d = new File(OUT + "/pp_done.txt"); d.open("w"); d.writeln(res.join(" ")); d.close();
