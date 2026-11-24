% gpu_matmul: gpuArray matrix products in single and double, compared with the same products on the CPU, then a fixed-count sustained phase.
% No timing criterion: every loop has a fixed iteration count; the host asserts GPU use from its own counters.
row = 'gpu_matmul';
try
    d = gpuDevice(1);
    rng(12345, 'twister');
    checks = {};
    types = {'single', 'double'};
    for t = 1:2
        dt = types{t};
        for n = [256 512 1024]
            A = rand(n, n, dt);
            B = rand(n, n, dt);
            Cg = gpuArray(A) * gpuArray(B);
            checks{end+1} = kf_cmp(sprintf('matmul_%s_%d', dt, n), Cg, A * B, kf_tol(dt)); %#ok<SAGROW>
        end
    end
    A = rand(4096, 4096, 'single');
    B = rand(4096, 4096, 'single');
    Ag = gpuArray(A);
    Bg = gpuArray(B);
    for k = 1:300
        Cg = Ag * Bg;
    end
    wait(d);
    checks{end+1} = kf_cmp('matmul_single_4096_after_300', Cg, A * B, kf_tol('single'));
    clear Ag Bg Cg A B
    A = rand(2048, 2048, 'double');
    B = rand(2048, 2048, 'double');
    Ag = gpuArray(A);
    Bg = gpuArray(B);
    for k = 1:60
        Cg = Ag * Bg;
    end
    wait(d);
    checks{end+1} = kf_cmp('matmul_double_2048_after_60', Cg, A * B, kf_tol('double'));
    doc = struct();
    doc.status = 'done';
    doc.checks = checks;
    kf_write(row, doc);
catch ME
    kf_fail(row, ME);
    rethrow(ME);
end
