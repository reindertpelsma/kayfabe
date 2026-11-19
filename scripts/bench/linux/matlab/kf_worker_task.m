function c = kf_worker_task(k, n)
% Runs on a parallel pool worker: a gpuArray matmul repeated for load, compared with the same product on the CPU.
rng(k, 'twister');
A = rand(n, n, 'single');
B = rand(n, n, 'single');
Ag = gpuArray(A);
Bg = gpuArray(B);
for r = 1:400
    Cg = Ag * Bg;
end
c = kf_cmp(sprintf('worker_task_%d', k), Cg, A * B, kf_tol('single'));
end
