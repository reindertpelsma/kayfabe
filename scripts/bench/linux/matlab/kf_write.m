function kf_write(row, doc)
% Write one row's result as JSON (text only, no paths, no account data). The host re-reads and re-judges every number.
doc.schema = 'kayfabe.matlab-row/1';
doc.row = row;
txt = jsonencode(doc);
f = fullfile(kf_outdir(), [row '.json']);
tmp = [f '.tmp'];
fid = fopen(tmp, 'w');
assert(fid > 0, 'kf:write');
fwrite(fid, txt, 'char');
fclose(fid);
movefile(tmp, f, 'f');
end
