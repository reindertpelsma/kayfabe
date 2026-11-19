% parpool_gpu: a local process pool of two workers, each selecting the GPU, then parfeval tasks that run gpuArray work and compare it with the CPU.
% On a one-GPU machine both workers share the device (two CUDA contexts on one die).
row = 'parpool_gpu';
pool = [];
try
    delete(gcp('nocreate'));
    pool = parpool('Processes', 2);
    here = fileparts(mfilename('fullpath'));
    addAttachedFiles(pool, {fullfile(here, 'kf_worker_task.m'), fullfile(here, 'kf_cmp.m'), fullfile(here, 'kf_fp.m'), fullfile(here, 'kf_tol.m')});
    spmd
        g = gpuDevice(1);
        wpid = 0;
        try
            wpid = feature('getpid');
        catch
        end
        winfo = [double(g.Index), double(wpid)];
    end
    pids = zeros(1, numel(winfo));
    idxs = zeros(1, numel(winfo));
    for i = 1:numel(winfo)
        w = winfo{i};
        idxs(i) = w(1);
        pids(i) = w(2);
    end
    checks = {};
    checks{end+1} = kf_atleast('num_workers', pool.NumWorkers, 2);
    checks{end+1} = kf_atleast('distinct_pids', numel(unique(pids(pids > 0))), 2);
    checks{end+1} = kf_atleast('workers_with_a_device', sum(idxs >= 1), 2);
    futs = cell(1, 4);
    for k = 1:4
        futs{k} = parfeval(pool, @kf_worker_task, 1, k, 2048);
    end
    for k = 1:4
        checks{end+1} = fetchOutputs(futs{k}); %#ok<SAGROW>
    end
    delete(pool);
    doc = struct();
    doc.status = 'done';
    doc.checks = checks;
    kf_write(row, doc);
catch ME
    kf_fail(row, ME);
    try
        delete(gcp('nocreate'));
    catch
    end
    rethrow(ME);
end
