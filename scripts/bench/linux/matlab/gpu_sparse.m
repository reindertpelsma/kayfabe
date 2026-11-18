% gpu_sparse: sparse gpuArrays (cuSPARSE and the sparse solvers) on a 2-D Laplacian: matrix-vector, complex vector, matrix-matrix, sparse-sparse product, construction
% from index vectors, row sums, diagonal, a 500-step power iteration, a direct sparse solve and a conjugate-gradient solve. Products against the CPU; solves by the
% residual computed on the CPU. Falsifier: a product misses the host's double tolerance, a residual exceeds 1e-10, or a sparse gpuArray operation errors.
row = 'gpu_sparse';
try
    d = gpuDevice(1);
    rng(51, 'twister');
    checks = {};
    try
        A = delsq(numgrid('S', 258));
        n = size(A, 1);
        Ag = gpuArray(A);
        if ~issparse(Ag)
            error('kf:sparse:notsparse', 'gpuArray(sparse) is not sparse');
        end
        checks{end+1} = kf_gpu('sparse_is_gpu_sparse', Ag);
        x = randn(n, 1);
        checks{end+1} = kf_cmp('spmv_double_65536', Ag * gpuArray(x), A * x, kf_tol('double'));
        z = complex(randn(n, 1), randn(n, 1));
        checks{end+1} = kf_cmp('spmv_complex_double_65536', Ag * gpuArray(z), A * z, kf_tol('double'));
        X = randn(n, 8);
        checks{end+1} = kf_cmp('spmm_double_8cols', Ag * gpuArray(X), A * X, kf_tol('double'));
        checks{end+1} = kf_cmp('spgemm_apply_double', (Ag * Ag) * gpuArray(x), (A * A) * x, kf_tol('double'));
        [i, j, v] = find(A);
        S = sparse(gpuArray(i), gpuArray(j), gpuArray(v), n, n);
        checks{end+1} = kf_cmp('sparse_build_apply_double', S * gpuArray(x), A * x, kf_tol('double'));
        checks{end+1} = kf_cmp('sparse_rowsum_double', full(sum(Ag, 2)), full(sum(A, 2)), kf_tol('double'));
        checks{end+1} = kf_cmp('sparse_diag_double', full(diag(Ag)), full(diag(A)), kf_tol('double'));
        checks{end+1} = kf_cmp('sparse_nnz_exact', full(sum(sum(Ag ~= 0))), nnz(A), kf_tol('double'));
        xg = gpuArray(x);
        xc = x;
        for k = 1:500
            xg = Ag * xg;
            xg = xg / norm(xg);
            xc = A * xc;
            xc = xc / norm(xc);
        end
        wait(d);
        checks{end+1} = kf_cmp('spmv_power500_double', xg, xc, kf_tol('double'));
        checks{end+1} = kf_count('sparse_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('sparse_status', ME);
    end
    try
        A4 = delsq(numgrid('S', 66));
        b4 = randn(size(A4, 1), 1);
        y4 = gather(gpuArray(A4) \ gpuArray(b4));
        checks{end+1} = kf_resid('sparse_mldivide_resid_double_4096', norm(A4 * y4 - b4) / norm(b4), 1e-10);
        A16 = delsq(numgrid('S', 130));
        b16 = randn(size(A16, 1), 1);
        [y16, flag] = pcg(gpuArray(A16), gpuArray(b16), 1e-12, 5000);
        checks{end+1} = kf_resid('pcg_resid_double_16384', norm(A16 * gather(y16) - b16) / norm(b16), 1e-10);
        checks{end+1} = kf_count('pcg_flag_nonzero', flag, 0);
        checks{end+1} = kf_count('solve_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('solve_status', ME);
    end
    doc = struct();
    doc.status = 'done';
    doc.checks = checks;
    kf_write(row, doc);
catch ME
    kf_fail(row, ME);
    rethrow(ME);
end
