// Photoshop AI selections through Photoshop's own action manager, each in its own try block; the error text is kept verbatim.
// Opens the data-volume photo, runs Select Subject (autoCutout, cloud off), Select Sky, Remove Background, and reports selection bounds.
app.displayDialogs = DialogModes.NO;
function sid(s) { return stringIDToTypeID(s); }
var out = [];
function T() { return new Date().toUTCString(); }
function sel(doc) { try { var b = doc.selection.bounds; return 'selection ' + b[0].as('px') + ',' + b[1].as('px') + ',' + b[2].as('px') + ',' + b[3].as('px'); } catch (e) { return 'no selection'; } }
var src = null, vols = 'DEFGHIJKLMNOPQRSTUVWXYZ';
for (var i = 0; i < vols.length && !src; i++) { var f = new File(vols.charAt(i) + ':/photo/ktiponissi_dirfys_4480x2928.jpg'); if (f.exists) src = f; }
var doc = null;
try { doc = app.open(src); out.push('open ok ' + src.name + ' ' + T()); } catch (e) { out.push('open ERR ' + e + ' src=' + src); }
function act(label, fn) {
  var t0 = new Date().getTime();
  try { doc.selection.deselect(); } catch (e) {}
  try { fn(); out.push(label + ' OK ' + sel(doc) + ' ms=' + (new Date().getTime() - t0) + ' at ' + T()); }
  catch (e) { out.push(label + ' ERR ms=' + (new Date().getTime() - t0) + ' at ' + T() + ' :: ' + String(e).replace(/[\r\n]+/g, ' ') + ' (number ' + e.number + ')'); }
}
if (doc) {
  act('select_subject_device', function () { var d = new ActionDescriptor(); d.putBoolean(sid('sampleAllLayers'), false); try { d.putEnumerated(sid('processing'), sid('processingType'), sid('device')); } catch (e) {} executeAction(sid('autoCutout'), d, DialogModes.NO); });
  act('select_subject_default', function () { var d = new ActionDescriptor(); d.putBoolean(sid('sampleAllLayers'), false); executeAction(sid('autoCutout'), d, DialogModes.NO); });
  act('select_sky', function () { executeAction(sid('selectSky'), new ActionDescriptor(), DialogModes.NO); });
  act('remove_background', function () { executeAction(sid('removeBackground'), new ActionDescriptor(), DialogModes.NO); });
}
out.join(' || ');
