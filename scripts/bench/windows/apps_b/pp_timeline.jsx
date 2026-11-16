// Premiere Pro (ExtendScript, run through the Startup stub): import the neutral timeline (FCPXML) into a new project, describe the sequence,
// then export 5-second slices: MODE "direct" uses the sequence's own exporter (sequence.exportAsMediaDirect, synchronous), MODE "ame" queues
// every job in Media Encoder (app.encoder.encodeSequence + startBatch; the host waits for the files). Tokens are replaced by the caller.
var OUT = "@OUT@", FCP = "@FCPXML@", MODE = "@MODE@", JOBS = @JOBS@;
function L(m) { try { var f = new File(OUT + "/pp_progress.txt"); f.open("a"); f.writeln(new Date().getTime() + " " + m); f.close(); } catch (e) {} }
L("begin " + app.version + " mode " + MODE);
var summary = [];
try {
  var made = app.newProject(OUT + "/kf.prproj");
  L("newProject " + made + " name=" + app.project.name);
  var bin = app.project.getInsertionBin();
  var ok = app.project.importFiles([FCP], true, bin, false);
  L("importFiles " + ok);
  var t0 = new Date().getTime();
  while (app.project.sequences.numSequences < 1 && new Date().getTime() - t0 < 120000) { $.sleep(2000); }
  var ns = app.project.sequences.numSequences;
  summary.push("sequences=" + ns);
  var seq = null;
  for (var i = 0; i < ns; i++) { if (!seq || /kayfabe/i.test(app.project.sequences[i].name)) seq = app.project.sequences[i]; }
  if (seq) {
    summary.push("seq=" + seq.name);
    try { summary.push("size=" + seq.frameSizeHorizontal + "x" + seq.frameSizeVertical); } catch (e) {}
    var vt = seq.videoTracks, at = seq.audioTracks, nclips = 0, ntr = 0;
    for (var v = 0; v < vt.numTracks; v++) { var c = vt[v].clips.numItems; nclips += c; if (c) ntr++; }
    var aclips = 0; for (var a = 0; a < at.numTracks; a++) { aclips += at[a].clips.numItems; }
    summary.push("vtracks=" + vt.numTracks + " vclips=" + nclips + " atracks=" + at.numTracks + " aclips=" + aclips);
    try { summary.push("end=" + seq.end); } catch (e) {}
    try { var tr = 0; for (var v2 = 0; v2 < vt.numTracks; v2++) tr += vt[v2].transitions.numItems; summary.push("transitions=" + tr); } catch (e) { summary.push("transitions_err"); }
    try { app.project.activeSequence = seq; } catch (e) {}
    L("sequence " + summary.join(" "));
    // 5 second slice from the start of the timeline
    try { seq.setInPoint(0); seq.setOutPoint(5); L("in/out set"); } catch (e) { L("inout ERR " + e); }
    if (MODE == "ame") { try { app.encoder.launchEncoder(); L("launchEncoder"); } catch (e) { L("launch ERR " + e); } }
    for (var j = 0; j < JOBS.length; j++) {
      var job = JOBS[j], res = "";
      try {
        var out = OUT + "/" + job[0] + "." + job[2];
        if (MODE == "direct") { res = seq.exportAsMediaDirect(out, job[1], app.encoder.ENCODE_IN_TO_OUT); }
        else { res = app.encoder.encodeSequence(seq, out, job[1], app.encoder.ENCODE_IN_TO_OUT, 1); }
        L("job " + job[0] + " -> " + res);
      } catch (e) { L("job " + job[0] + " ERR " + e); }
    }
    if (MODE == "ame") { try { app.encoder.startBatch(); L("startBatch"); } catch (e) { L("startBatch ERR " + e); } }
  } else { L("no sequence after import"); }
} catch (e) { L("TOP ERR " + e); }
var d = new File(OUT + "/pp_done.txt"); d.open("w"); d.writeln(summary.join(" | ")); d.close();
