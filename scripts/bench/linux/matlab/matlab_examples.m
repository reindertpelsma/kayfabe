% matlab_examples: production workloads, not synthetic ones: MATLAB's own SHIPPED examples (discovered in the installed products' example folders and run non-interactively in a
% single-worker pool with a time budget each, a timeout kills the worker mid-kernel and the next example gets a fresh pool), the product's own FEATURE LISTS swept function by function
% (the methods of gpuArray, the deep-learning layer constructors, every solver by every execution environment of trainingOptions) with the CPU result as oracle, error paths compared
% by identifier class, and messy variants (memory near the limit with an out-of-memory error and recovery, odd shapes, a rerun after reset(gpuDevice), a worker killed mid-kernel, a soak).
% Every part reports its own status; the lists of failing names are evidence, the counts are judged by the host (host minimums, zero for every failure count).
% Falsifier: any failure count is nonzero, a sweep tests fewer functions than the host's minimum, an error path raises a different identifier class than the host's, or a messy variant changes
% a result. Oracle for the shipped examples is outcome only (ran or threw, with the CPU-only control for the ones that threw); there is no numeric oracle for an arbitrary example.
row = 'matlab_examples';
try
    d = gpuDevice(1);
    checks = {};
    doc = struct();
    here = fileparts(mfilename('fullpath'));
    cfg = jsondecode(fileread(fullfile(here, 'kf_examples.json')));
    outdir = fullfile(kf_outdir(), 'examples');
    enum = struct();
    % ---- 1. shipped examples
    failed = {};
    chosen = {};
    names = {};
    try
        found = {};
        for r = 1:numel(cfg.example_roots)
            hits = dir(fullfile(matlabroot, 'examples', cfg.example_roots{r}, '**', '*.mlx'));
            for k = 1:numel(hits)
                found{end+1} = fullfile(hits(k).folder, hits(k).name); %#ok<SAGROW>
            end
        end
        [~, base] = cellfun(@fileparts, found, 'UniformOutput', false);
        keep = true(size(found));
        for k = 1:numel(cfg.example_deny)
            keep = keep & ~contains(base, cfg.example_deny{k});
        end
        found = found(keep);
        base = base(keep);
        score = zeros(size(found));
        for k = 1:numel(cfg.example_prefer)
            score = score + contains(base, cfg.example_prefer{k});
        end
        [~, order] = sort(score, 'descend');
        chosen = found(order(1:min(cfg.example_max, numel(order))));
        names = base(order(1:min(cfg.example_max, numel(order))));
        timeouts = 0;
        ran = 0;
        pool = parpool('Processes', 1);
        for k = 1:numel(chosen)
            f = parfeval(pool, @kf_run_example, 1, chosen{k}, outdir);
            if ~wait(f, 'finished', cfg.example_budget_s)
                cancel(f);
                timeouts = timeouts + 1;
                failed{end+1} = [names{k} ':timeout']; %#ok<SAGROW>
                delete(pool);
                pool = parpool('Processes', 1);
                continue
            end
            ran = ran + 1;
            if ~isempty(f.Error)
                failed{end+1} = [names{k} ':worker']; %#ok<SAGROW>
            else
                res = fetchOutputs(f);
                if ~res.ok
                    failed{end+1} = [names{k} ':' res.err]; %#ok<SAGROW>
                end
            end
        end
        delete(pool);
        enum.examples_failed = failed(1:min(numel(failed), 40));
        checks{end+1} = kf_atleast('examples_discovered', numel(found), 20);
        checks{end+1} = kf_atleast('examples_run', ran, 10);
        checks{end+1} = kf_count('examples_failed', numel(failed), 0);
        checks{end+1} = kf_count('examples_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('examples_status', ME);
        try
            delete(gcp('nocreate'));
        catch
        end
    end
    doc.status = 'partial';
    doc.checks = checks;
    doc.enum = enum;
    kf_write(row, doc);       % a time-boxed kill must not lose the parts that finished (the host reads only a 'done' document; the partial file is for the evidence pull)
    % ---- 2. feature enumeration
    try
        s = kf_enum_gpuarray(cfg);
        checks{end+1} = kf_atleast('enum_gpuarray_tested', s.tested, 100);
        checks{end+1} = kf_count('enum_gpuarray_diff', s.diff, 0);
        checks{end+1} = kf_count('enum_gpuarray_gpu_only_error', s.gpu_only_error, 0);
        checks{end+1} = kf_range('enum_gpuarray_pass_fraction', s.pass / max(s.tested, 1), 0.90, 1.0);
        checks{end+1} = kf_count('enum_gpuarray_status', 0, 0);
        enum.gpuarray_diff = s.names_diff(1:min(numel(s.names_diff), 40));
        enum.gpuarray_gpu_only_error = s.names_gpu_only_error(1:min(numel(s.names_gpu_only_error), 40));
        enum.gpuarray_counts = [s.tested s.pass s.diff s.gpu_only_error s.both_error s.nonnumeric s.skipped];
    catch ME
        checks{end+1} = kf_err('enum_gpuarray_status', ME);
    end
    try
        s = kf_enum_layers(cfg);
        checks{end+1} = kf_atleast('enum_layers_tested', s.tested, 20);
        checks{end+1} = kf_count('enum_layers_diff', s.diff, 0);
        checks{end+1} = kf_count('enum_layers_gpu_only_error', s.gpu_only_error, 0);
        checks{end+1} = kf_count('enum_layers_status', 0, 0);
        enum.layers_diff = s.names_diff(1:min(numel(s.names_diff), 40));
        enum.layers_gpu_only_error = s.names_gpu_only_error(1:min(numel(s.names_gpu_only_error), 40));
        enum.layers_both_error = s.names_both_error(1:min(numel(s.names_both_error), 40));
        enum.layers_counts = [s.tested s.pass s.diff s.gpu_only_error s.both_error];
    catch ME
        checks{end+1} = kf_err('enum_layers_status', ME);
    end
    try
        s = kf_enum_train(cfg);
        checks{end+1} = kf_atleast('enum_train_cells', s.cells, 24);
        checks{end+1} = kf_count('enum_train_diff', s.diff, 0);
        checks{end+1} = kf_count('enum_train_unexpected_error', s.unexpected_error, 0);
        checks{end+1} = kf_count('enum_train_status', 0, 0);
        enum.train_diff = s.names_diff(1:min(numel(s.names_diff), 40));
        enum.train_unexpected_error = s.names_unexpected_error(1:min(numel(s.names_unexpected_error), 40));
        enum.train_refused = s.names_refused(1:min(numel(s.names_refused), 40));
        enum.train_counts = [s.cells s.pass s.diff s.unexpected_error s.refused];
    catch ME
        checks{end+1} = kf_err('enum_train_status', ME);
    end
    doc.status = 'partial';
    doc.checks = checks;
    doc.enum = enum;
    kf_write(row, doc);       % a time-boxed kill must not lose the parts that finished (the host reads only a 'done' document; the partial file is for the evidence pull)
    % ---- 3. error paths
    try
        free0 = d.AvailableMemory;
        checks{end+1} = kf_errpath('errpath_innerdim', @() ones(2, 3) * ones(2, 3), @() gpuArray(ones(2, 3)) * gpuArray(ones(2, 3)));
        checks{end+1} = kf_errpath('errpath_badindex', @() subsref(ones(3), struct('type', '()', 'subs', {{4, 4}})), @() subsref(gpuArray(ones(3)), struct('type', '()', 'subs', {{4, 4}})));
        checks{end+1} = kf_errpath('errpath_oom', [], @() zeros(ceil(sqrt(4 * free0 / 8)), ceil(sqrt(4 * free0 / 8)), 'gpuArray'));
        checks{end+1} = kf_errpath('errpath_wrongclass', [], @() gpuArray({1, 2}));
        checks{end+1} = kf_errpath('errpath_badptx', [], @() parallel.gpu.CUDAKernel(fullfile(here, 'kf_examples.json'), 'float *', 'nothing'));
        checks{end+1} = kf_count('errpath_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('errpath_status', ME);
    end
    doc.status = 'partial';
    doc.checks = checks;
    doc.enum = enum;
    kf_write(row, doc);       % a time-boxed kill must not lose the parts that finished (the host reads only a 'done' document; the partial file is for the evidence pull)
    % ---- 4. messy variants
    try
        free0 = d.AvailableMemory;
        chunk = 128 * 2 ^ 20;
        held = {};
        for k = 1:min(floor(0.92 * free0 / chunk), 512)
            held{end+1} = zeros(chunk / 4, 1, 'single', 'gpuArray'); %#ok<SAGROW>
        end
        frac = numel(held) * chunk / free0;
        A = rand(256, 256, 'single');
        Cg = gpuArray(A) * gpuArray(A);
        checks{end+1} = kf_cmp('messy_under_pressure_single', Cg, A * A, kf_tol('single'));
        checks{end+1} = kf_range('messy_vram_fraction', frac, 0.70, 0.95);
        oom = false;
        try
            more = zeros(ceil(0.25 * free0 / 4), 1, 'single', 'gpuArray'); %#ok<NASGU>
        catch
            oom = true;
        end
        clear held more
        wait(d);
        B = rand(2048, 2048, 'single');
        checks{end+1} = kf_cmp('messy_oom_recovery_single', gpuArray(B) * gpuArray(B), B * B, kf_tol('single'));
        checks{end+1} = kf_count('messy_no_oom_raised', double(~oom), 0);
        sizes = {[1 1], [1 7], [7 1], [0 5], [5 0], [31 33], [1023 1025], [2047 1], [1 4099], [127 131]};
        bad = 0;
        for k = 1:numel(sizes)
            X = rand(sizes{k});
            Xg = gpuArray(X);
            r1 = gather(sum(Xg, 1) + max(Xg(:), [], 1, 'omitnan')) - (sum(X, 1) + max(X(:), [], 1, 'omitnan'));
            bad = bad + double(~isempty(r1) && norm(r1(:)) > 1e-9 * max(1, norm(sum(X, 1))));
        end
        checks{end+1} = kf_count('messy_odd_shapes_mismatches', bad, 0);
        gpurng(5, 'Philox4x32-10');
        u1 = gather(rand(1e6, 1, 'gpuArray'));
        reset(d);
        d = gpuDevice(1);
        gpurng(5, 'Philox4x32-10');
        u2 = rand(1e6, 1, 'gpuArray');
        checks{end+1} = kf_count('messy_rerun_after_reset_mismatches', sum(u1 ~= gather(u2)), 0);
        pool = parpool('Processes', 1);
        f = parfeval(pool, @kf_long_kernel, 1, 100000);
        started = wait(f, 'running', 60);
        cancel(f);
        delete(pool);
        checks{end+1} = kf_count('messy_worker_not_running_when_killed', double(~started), 0);
        E1 = rand(1024, 1024);
        checks{end+1} = kf_cmp('messy_kill_recover_double', gpuArray(E1) * gpuArray(E1), E1 * E1, kf_tol('double'));
        pool = parpool('Processes', 1);
        g = parfeval(pool, @(m) gather(gpuArray(m) * gpuArray(m)), 1, E1);
        checks{end+1} = kf_cmp('messy_new_pool_after_kill_double', gpuArray(fetchOutputs(g)), E1 * E1, kf_tol('double'));
        delete(pool);
        S = rand(2048, 2048, 'single');
        Sg = gpuArray(S);
        acc = zeros(1, 1, 'single', 'gpuArray');
        for k = 1:6000
            T = Sg * Sg;
            acc = acc + T(1, 1) / numel(T);
        end
        wait(d);
        T0 = S * S;
        checks{end+1} = kf_cmp('messy_soak_single_after_6000', T, T0, kf_tol('single'));
        checks{end+1} = kf_count('messy_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('messy_status', ME);
        try
            delete(gcp('nocreate'));
        catch
        end
    end
    doc.status = 'partial';
    doc.checks = checks;
    doc.enum = enum;
    kf_write(row, doc);       % a time-boxed kill must not lose the parts that finished (the host reads only a 'done' document; the partial file is for the evidence pull)
    % ---- 5. the CPU-only control of the examples that threw, LAST: the workers it starts have no visible GPU (the variable is left set; nothing after this needs a GPU worker)
    try
        cpu_ok = 0;
        enum.examples_failed_cpu_ok = {};
        if ~isempty(failed)
            setenv('CUDA_VISIBLE_DEVICES', '');
            pool = parpool('Processes', 1);
            for k = 1:numel(chosen)
                if any(startsWith(failed, [names{k} ':']))
                    f = parfeval(pool, @kf_run_example, 1, chosen{k}, outdir);
                    if wait(f, 'finished', cfg.example_budget_s)
                        if isempty(f.Error) && fetchOutputs(f).ok
                            cpu_ok = cpu_ok + 1;
                            enum.examples_failed_cpu_ok{end+1} = names{k};
                        end
                    else
                        cancel(f);
                        delete(pool);
                        pool = parpool('Processes', 1);
                    end
                end
            end
            delete(pool);
        end
        checks{end+1} = kf_count('examples_failed_cpu_ok', cpu_ok, 0);
        checks{end+1} = kf_count('examples_control_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('examples_control_status', ME);
    end
    doc.status = 'done';
    doc.checks = checks;
    doc.enum = enum;
    kf_write(row, doc);
catch ME
    kf_fail(row, ME);
    rethrow(ME);
end
