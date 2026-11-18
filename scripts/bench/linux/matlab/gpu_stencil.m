% gpu_stencil: launch-rate stress. An explicit 2-D heat equation (1500 steps of nine tiny kernels each, both precisions), a 300-step conv2 smoother, and a fixed-step
% RK4 integration of 4096 harmonic oscillators (10000 steps, about 35 element-wise kernels per step: several hundred thousand launches). GPU against the same loop on the CPU;
% the RK4 result is also checked against the analytic solution. Falsifier: any final state misses the host's tolerance, or the launch stream stalls (the row's time-box).
row = 'gpu_stencil';
try
    d = gpuDevice(1);
    rng(71, 'twister');
    checks = {};
    try
        types = {'double', 'single'};
        for t = 1:2
            dt = types{t};
            u0 = rand(512, 512, dt);
            r = cast(0.2, dt);
            ug = gpuArray(u0);
            uc = u0;
            for k = 1:1500
                ug = ug + r * (circshift(ug, 1, 1) + circshift(ug, -1, 1) + circshift(ug, 1, 2) + circshift(ug, -1, 2) - 4 * ug);
                uc = uc + r * (circshift(uc, 1, 1) + circshift(uc, -1, 1) + circshift(uc, 1, 2) + circshift(uc, -1, 2) - 4 * uc);
            end
            wait(d);
            checks{end+1} = kf_cmp(sprintf('heat_circshift_%s_512', dt), ug, uc, kf_tol(dt)); %#ok<SAGROW>
        end
        K = [1 2 1; 2 4 2; 1 2 1] / 16;
        u0 = rand(512, 512);
        ug = gpuArray(u0);
        uc = u0;
        for k = 1:300
            ug = conv2(ug, K, 'same');
            uc = conv2(uc, K, 'same');
        end
        checks{end+1} = kf_cmp('conv2_same_double_512', ug, uc, kf_tol('double'));
        checks{end+1} = kf_count('stencil_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('stencil_status', ME);
    end
    try
        m = 4096;
        w = linspace(1, 2, m)';
        x0 = cos(linspace(0, 6, m))';
        dt = 1e-3;
        steps = 10000;
        res = cell(1, 2);
        for e = 1:2
            w2 = w .^ 2;
            x = x0;
            v = zeros(m, 1);
            if e == 1
                w2 = gpuArray(w2);
                x = gpuArray(x);
                v = gpuArray(v);
            end
            for k = 1:steps
                k1x = v;
                k1v = -w2 .* x;
                k2x = v + 0.5 * dt * k1v;
                k2v = -w2 .* (x + 0.5 * dt * k1x);
                k3x = v + 0.5 * dt * k2v;
                k3v = -w2 .* (x + 0.5 * dt * k2x);
                k4x = v + dt * k3v;
                k4v = -w2 .* (x + dt * k3x);
                x = x + dt / 6 * (k1x + 2 * k2x + 2 * k3x + k4x);
                v = v + dt / 6 * (k1v + 2 * k2v + 2 * k3v + k4v);
            end
            res{e} = x;
        end
        wait(d);
        checks{end+1} = kf_cmp('rk4_double_4096', res{1}, res{2}, kf_tol('double'));
        checks{end+1} = kf_resid('rk4_analytic_resid_double_4096', max(abs(gather(res{1}) - x0 .* cos(w * steps * dt))), 1e-10);
        checks{end+1} = kf_count('rk4_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('rk4_status', ME);
    end
    doc = struct();
    doc.status = 'done';
    doc.checks = checks;
    kf_write(row, doc);
catch ME
    kf_fail(row, ME);
    rethrow(ME);
end
