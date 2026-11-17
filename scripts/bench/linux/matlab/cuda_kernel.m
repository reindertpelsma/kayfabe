% cuda_kernel: parallel.gpu.CUDAKernel from a committed PTX text file (vecadd.ptx, JIT-compiled by the driver; no nvcc is needed). Element-wise
% float addition is exact, so the tolerance is zero. Then a fixed-count launch loop, and a check that nothing is written past the end.
row = 'cuda_kernel';
try
    d = gpuDevice(1);
    here = fileparts(mfilename('fullpath'));
    k = parallel.gpu.CUDAKernel(fullfile(here, 'vecadd.ptx'), 'const float *, const float *, float *, int', 'vecadd');
    k.ThreadBlockSize = 256;
    rng(12345, 'twister');
    checks = {};
    for n = [1000 1048573 10000000]
        k.GridSize = ceil(n / 256);
        a = rand(n, 1, 'single');
        b = rand(n, 1, 'single');
        ag = gpuArray(a);
        bg = gpuArray(b);
        cg = zeros(n, 1, 'single', 'gpuArray');
        cg = feval(k, ag, bg, cg, int32(n));
        checks{end+1} = kf_cmp(sprintf('vecadd_%d', n), cg, a + b, 0); %#ok<SAGROW>
    end
    for r = 1:40000
        cg = feval(k, ag, bg, cg, int32(n));
    end
    wait(d);
    checks{end+1} = kf_cmp('vecadd_after_40000_launches', cg, a + b, 0);
    m = 4096;
    k.GridSize = ceil((m + 3000) / 256);
    a = rand(m + 3000, 1, 'single');
    b = rand(m + 3000, 1, 'single');
    sentinel = -7 * ones(m + 3000, 1, 'single');
    og = feval(k, gpuArray(a), gpuArray(b), gpuArray(sentinel), int32(m));
    oh = gather(og);
    checks{end+1} = kf_count('no_write_past_n', sum(oh(m+1:end) ~= -7), 0);
    checks{end+1} = kf_cmp('vecadd_first_n_of_guarded', oh(1:m), a(1:m) + b(1:m), 0);
    doc = struct();
    doc.status = 'done';
    doc.checks = checks;
    kf_write(row, doc);
catch ME
    kf_fail(row, ME);
    rethrow(ME);
end
