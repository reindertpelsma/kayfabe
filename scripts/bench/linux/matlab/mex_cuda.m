% mex_cuda: a real CUDA C++ MEX function compiled in the guest with mexcuda (the nvcc MATLAB brings, the guest's gcc as host compiler; no CUDA Toolkit is installed in the image):
% shared-memory histogram with global atomics and a shared-memory tree reduction with a double atomicAdd, on three sizes and 2000 launches, results exact against the CPU.
% The compile touches no device; a failure there is a toolchain matter (host compiler, bundled nvcc) and is reported as its own step. A failure of the run step is a device matter.
% Falsifier: a histogram bin or the sum differs in one count, or the compiled function cannot launch.
row = 'mex_cuda';
try
    d = gpuDevice(1);
    rng(81, 'twister');
    checks = {};
    here = fileparts(mfilename('fullpath'));
    wd = fullfile(kf_outdir(), 'mexwork');
    if ~isfolder(wd)
        mkdir(wd);
    end
    copyfile(fullfile(here, 'kf_mex.cu'), wd);
    compiled = false;
    try
        mexcuda('-outdir', wd, fullfile(wd, 'kf_mex.cu'));
        addpath(wd);
        rehash path;
        compiled = true;
        checks{end+1} = kf_count('mexcuda_compile_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('mexcuda_compile_status', ME);
    end
    try
        if ~compiled
            error('kf:mex:notcompiled', 'not compiled');
        end
        small = uint8(randi(256, 1000003, 1) - 1);
        large = uint8(randi(256, 33554432, 1) - 1);
        bs = kf_mex('hist', gpuArray(small));
        checks{end+1} = kf_gpu('mex_output_gpuarray', bs);
        checks{end+1} = kf_cmp('exact_mex_hist_small', bs, accumarray(double(small) + 1, 1, [256 1]), 0);
        bl = kf_mex('hist', gpuArray(large));
        checks{end+1} = kf_cmp('exact_mex_hist_large', bl, accumarray(double(large) + 1, 1, [256 1]), 0);
        checks{end+1} = kf_count('mex_hist_total_count', sum(double(gather(bl))) - numel(large), 0);
        x = round(rand(2000003, 1) * 100);
        checks{end+1} = kf_cmp('exact_mex_sum', kf_mex('sum', gpuArray(x)), sum(x), 0);
        gs = gpuArray(small);
        for r = 1:2000
            bs = kf_mex('hist', gs);
        end
        wait(d);
        checks{end+1} = kf_cmp('exact_mex_hist_after_2000_launches', bs, accumarray(double(small) + 1, 1, [256 1]), 0);
        checks{end+1} = kf_count('mex_run_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('mex_run_status', ME);
    end
    doc = struct();
    doc.status = 'done';
    doc.checks = checks;
    kf_write(row, doc);
catch ME
    kf_fail(row, ME);
    rethrow(ME);
end
