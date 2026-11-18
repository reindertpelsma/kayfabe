function r = kf_run_example(path, workdir)
% Runs one shipped example (a live script) non-interactively in the current process (a pool worker): in a scratch folder, figures closed afterwards, the error
% identifier only (never the message) when it throws. Returns a struct with ok and err.
r = struct('ok', false, 'err', '');
try
    if ~isfolder(workdir)
        mkdir(workdir);
    end
    cd(workdir);
    evalc('run(path)');
    r.ok = true;
catch ME
    r.err = ME.identifier;
    if isempty(r.err)
        r.err = 'kf:noid';
    end
end
try
    close all force
catch
end
end
