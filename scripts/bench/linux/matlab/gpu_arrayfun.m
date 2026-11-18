% gpu_arrayfun: element-wise kernel fusion as people use it (arrayfun on gpuArrays: a fused expression, a loop-and-branch kernel in a file, two outputs, integer and
% logical kernels, implicit expansion) against the same expression evaluated on the CPU. Tolerances come from kf_tol; integer and logical results are exact.
% Falsifier: any element of any result differs beyond the host's tolerance, or arrayfun cannot compile or launch a kernel.
row = 'gpu_arrayfun';
try
    d = gpuDevice(1);
    rng(21, 'twister');
    checks = {};
    n = 2048;
    try
        types = {'single', 'double'};
        for t = 1:2
            dt = types{t};
            X = randn(n, n, dt);
            Y = randn(n, n, dt);
            a = cast(0.75, dt);
            f = @(x, y, a) a .* sin(x) .^ 2 + cos(y) .* exp(-x .^ 2) - log1p(abs(y)) + erf(x .* y) + hypot(x, y) ./ (1 + atan2(y, x) .^ 2) + tanh(a .* x - y);
            Zg = arrayfun(f, gpuArray(X), gpuArray(Y), a);
            checks{end+1} = kf_cmp(sprintf('af_expr_%s_4M', dt), Zg, f(X, Y, a), kf_tol(dt)); %#ok<SAGROW>
        end
        clear X Y Zg
        checks{end+1} = kf_count('af_expr_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('af_expr_status', ME);
    end
    try
        m = 1000;
        X0 = randn(m, m);
        C0 = rand(m, m);
        Y = X0;
        for k = 1:50
            Y = 0.5 * cos(Y) + C0;
        end
        Yg = arrayfun(@kf_af_iter, gpuArray(X0), gpuArray(C0));
        checks{end+1} = kf_cmp('af_loop_double_1M', Yg, Y, kf_tol('double'));
        checks{end+1} = kf_count('af_loop_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('af_loop_status', ME);
    end
    try
        Z = zeros(m, m);
        hi = Y > C0;
        Z(hi) = sqrt(Y(hi) - C0(hi));
        Z(~hi) = -log1p(C0(~hi) - Y(~hi));
        [Yg, Zg] = arrayfun(@kf_af_iter, gpuArray(X0), gpuArray(C0));
        checks{end+1} = kf_cmp('af_multiout_y_double', Yg, Y, kf_tol('double'));
        checks{end+1} = kf_cmp('af_multiout_z_double', Zg, Z, kf_tol('double'));
        Xs = single(X0);
        Cs = single(C0);
        Ys = Xs;
        for k = 1:50
            Ys = 0.5 * cos(Ys) + Cs;
        end
        Zs = zeros(m, m, 'single');
        hi = Ys > Cs;
        Zs(hi) = sqrt(Ys(hi) - Cs(hi));
        Zs(~hi) = -log1p(Cs(~hi) - Ys(~hi));
        [~, Zsg] = arrayfun(@kf_af_iter, gpuArray(Xs), gpuArray(Cs));
        checks{end+1} = kf_cmp('af_branch_single', Zsg, Zs, kf_tol('single'));
        checks{end+1} = kf_count('af_multiout_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('af_multiout_status', ME);
    end
    try
        A = randn(2000, 2000);
        b = randn(2000, 1);
        r = randn(1, 2000);
        Ag = gpuArray(A);
        Zg = Ag + gpuArray(b) .* gpuArray(r) - max(Ag, gpuArray(b));
        checks{end+1} = kf_cmp('implicit_expand_double', Zg, A + b .* r - max(A, b), kf_tol('double'));
    catch ME
        checks{end+1} = kf_err('implicit_expand_status', ME);
    end
    try
        x = randi([-100000 100000], 1000, 1000, 'int32');
        y = randi([-100000 100000], 1000, 1000, 'int32');
        fi = @(x, y) rem(x .* int32(3) + y, int32(1000)) + abs(x - y);
        checks{end+1} = kf_cmp('exact_af_int32', arrayfun(fi, gpuArray(x), gpuArray(y)), fi(x, y), 0);
        xs = rand(1000, 1000);
        ys = rand(1000, 1000);
        fl = @(x, y) (x > y) & ~(x > 0.5) | xor(x < 0.25, y < 0.75);
        checks{end+1} = kf_cmp('exact_af_logical', arrayfun(fl, gpuArray(xs), gpuArray(ys)), fl(xs, ys), 0);
        checks{end+1} = kf_count('af_int_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('af_int_status', ME);
    end
    try
        X = randn(2048, 2048, 'single');
        Y = randn(2048, 2048, 'single');
        g = @(x, y) tanh(0.9 * x + 0.1 * y);
        Xg = gpuArray(X);
        Yg = gpuArray(Y);
        Xc = X;
        for k = 1:200
            Xg = arrayfun(g, Xg, Yg);
            Xc = g(Xc, Y);
        end
        wait(d);
        checks{end+1} = kf_cmp('af_sustained_single_after_200', Xg, Xc, kf_tol('single'));
        checks{end+1} = kf_count('af_sustained_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('af_sustained_status', ME);
    end
    doc = struct();
    doc.status = 'done';
    doc.checks = checks;
    kf_write(row, doc);
catch ME
    kf_fail(row, ME);
    rethrow(ME);
end
