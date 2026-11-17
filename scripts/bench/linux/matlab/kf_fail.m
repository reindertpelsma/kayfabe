function kf_fail(row, ME)
% A row that threw: record only the error identifier (never the message: it may carry paths or vendor text), then the caller rethrows.
doc = struct();
doc.status = 'error';
doc.error_id = ME.identifier;
doc.checks = {};
try
    kf_write(row, doc);
catch
end
end
