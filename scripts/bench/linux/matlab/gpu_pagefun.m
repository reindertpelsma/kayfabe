% gpu_pagefun: batched small-matrix operations (pagemtimes with transposes and implicit expansion, pagefun(@mtimes), pagetranspose, pagectranspose, pagesvd)
% against the CPU, then 300 batched products. Transposes are exact; products and singular values use the host's tolerances.
% Falsifier: a page of any result differs beyond tolerance, or a batched library call fails.
row = 'gpu_pagefun';
try
    d = gpuDevice(1);
    rng(31, 'twister');
    checks = {};
    try
        types = {'single', 'double'};
        for t = 1:2
            dt = types{t};
            A = rand(32, 32, 4096, dt);
            B = rand(32, 32, 4096, dt);
            Cg = pagemtimes(gpuArray(A), gpuArray(B));
            checks{end+1} = kf_cmp(sprintf('pagemtimes_%s_32', dt), Cg, pagemtimes(A, B), kf_tol(dt)); %#ok<SAGROW>
        end
        A = rand(32, 32, 4096, 'single');
        B = rand(32, 32, 4096, 'single');
        Cg = pagemtimes(gpuArray(A), 'transpose', gpuArray(B), 'none');
        checks{end+1} = kf_cmp('pagemtimes_tn_single', Cg, pagemtimes(A, 'transpose', B, 'none'), kf_tol('single'));
        A = rand(32, 32, 512);
        B = rand(32, 32);
        Cg = pagemtimes(gpuArray(A), gpuArray(B));
        checks{end+1} = kf_cmp('pagemtimes_expand_double', Cg, pagemtimes(A, B), kf_tol('double'));
        checks{end+1} = kf_count('pagemtimes_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('pagemtimes_status', ME);
    end
    try
        A = rand(24, 24, 2048, 'single');
        B = rand(24, 24, 2048, 'single');
        Cg = pagefun(@mtimes, gpuArray(A), gpuArray(B));
        checks{end+1} = kf_cmp('pagefun_mtimes_single', Cg, pagemtimes(A, B), kf_tol('single'));
        checks{end+1} = kf_count('pagefun_mtimes_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('pagefun_mtimes_status', ME);
    end
    try
        A = rand(40, 17, 2048);
        checks{end+1} = kf_cmp('exact_pagetranspose_double', pagetranspose(gpuArray(A)), pagetranspose(A), 0);
        Z = complex(rand(40, 17, 1024), rand(40, 17, 1024));
        checks{end+1} = kf_cmp('exact_pagectranspose_complex', pagectranspose(gpuArray(Z)), pagectranspose(Z), 0);
        checks{end+1} = kf_count('pagetranspose_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('pagetranspose_status', ME);
    end
    try
        types = {'single', 'double'};
        for t = 1:2
            dt = types{t};
            A = rand(16, 16, 2048, dt);
            Sg = pagesvd(gpuArray(A));
            checks{end+1} = kf_cmp(sprintf('pagesvd_%s', dt), Sg, pagesvd(A), kf_tol(dt)); %#ok<SAGROW>
        end
        checks{end+1} = kf_count('pagesvd_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('pagesvd_status', ME);
    end
    try
        A = rand(64, 64, 1024, 'single');
        B = rand(64, 64, 1024, 'single');
        Ag = gpuArray(A);
        Bg = gpuArray(B);
        for k = 1:300
            Cg = pagemtimes(Ag, Bg);
        end
        wait(d);
        checks{end+1} = kf_cmp('pagemtimes_single_after_300', Cg, pagemtimes(A, B), kf_tol('single'));
        checks{end+1} = kf_count('pagemtimes_sustained_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('pagemtimes_sustained_status', ME);
    end
    doc = struct();
    doc.status = 'done';
    doc.checks = checks;
    kf_write(row, doc);
catch ME
    kf_fail(row, ME);
    rethrow(ME);
end
