% gpu_solve: mldivide, lu, qr and chol on gpuArray; solutions compared with the CPU, factorisations judged by residuals evaluated on the CPU.
row = 'gpu_solve';
try
    d = gpuDevice(1);
    rng(12345, 'twister');
    checks = {};
    types = {'single', 'double'};
    for t = 1:2
        dt = types{t};
        tol = kf_tol(dt);
        for n = [512 1024]
            A = rand(n, n, dt) + n * eye(n, dt);
            b = rand(n, 3, dt);
            Ag = gpuArray(A);
            xg = Ag \ gpuArray(b);
            xc = A \ b;
            xh = gather(xg);
            checks{end+1} = kf_cmp(sprintf('mldivide_%s_%d', dt, n), xg, xc, tol); %#ok<SAGROW>
            checks{end+1} = kf_resid(sprintf('mldivide_resid_gpu_%s_%d', dt, n), norm(A * xh - b) / (norm(A) * norm(xh) + norm(b)), tol); %#ok<SAGROW>
            checks{end+1} = kf_resid(sprintf('mldivide_resid_cpu_%s_%d', dt, n), norm(A * xc - b) / (norm(A) * norm(xc) + norm(b)), tol); %#ok<SAGROW>
            [Lg, Ug, Pg] = lu(Ag);
            checks{end+1} = kf_resid(sprintf('lu_resid_gpu_%s_%d', dt, n), norm(gather(Pg * Ag - Lg * Ug), 'fro') / norm(A, 'fro'), tol); %#ok<SAGROW>
            [Lc, Uc, Pc] = lu(A);
            checks{end+1} = kf_resid(sprintf('lu_resid_cpu_%s_%d', dt, n), norm(Pc * A - Lc * Uc, 'fro') / norm(A, 'fro'), tol); %#ok<SAGROW>
            [Qg, Rg] = qr(Ag);
            checks{end+1} = kf_resid(sprintf('qr_resid_gpu_%s_%d', dt, n), norm(gather(Qg * Rg - Ag), 'fro') / norm(A, 'fro'), tol); %#ok<SAGROW>
            checks{end+1} = kf_resid(sprintf('qr_orth_gpu_%s_%d', dt, n), norm(gather(Qg' * Qg - eye(n, dt, 'gpuArray')), 'fro') / sqrt(n), tol); %#ok<SAGROW>
            [Qc, Rc] = qr(A);
            checks{end+1} = kf_resid(sprintf('qr_resid_cpu_%s_%d', dt, n), norm(Qc * Rc - A, 'fro') / norm(A, 'fro'), tol); %#ok<SAGROW>
            S = A' * A + n * eye(n, dt);
            Sg = gpuArray(S);
            Rcg = chol(Sg);
            checks{end+1} = kf_resid(sprintf('chol_resid_gpu_%s_%d', dt, n), norm(gather(Rcg' * Rcg - Sg), 'fro') / norm(S, 'fro'), tol); %#ok<SAGROW>
        end
        At = rand(2000, 500, dt);
        bt = rand(2000, 1, dt);
        checks{end+1} = kf_cmp(sprintf('lstsq_%s_2000x500', dt), gpuArray(At) \ gpuArray(bt), At \ bt, tol); %#ok<SAGROW>
    end
    A = rand(2048, 2048, 'double') + 2048 * eye(2048, 'double');
    b = rand(2048, 1, 'double');
    Ag = gpuArray(A);
    bg = gpuArray(b);
    for k = 1:100
        xg = Ag \ bg;
    end
    wait(d);
    checks{end+1} = kf_cmp('mldivide_double_2048_after_100', xg, A \ b, kf_tol('double'));
    doc = struct();
    doc.status = 'done';
    doc.checks = checks;
    kf_write(row, doc);
catch ME
    kf_fail(row, ME);
    rethrow(ME);
end
