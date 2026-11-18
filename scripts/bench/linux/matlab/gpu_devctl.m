% gpu_devctl: device control as applications use it: device count and table, availability, a CUDA context torn down with reset(gpuDevice) and re-created four times with
% a matmul each time, stale arrays after the reset, the memory returned by the reset, wait and gputimeit. No timing criterion: gputimeit only has to return a positive finite number.
% Falsifier: an array survives a reset, memory is not returned by it, a re-created context computes a wrong product, or the device cannot be re-selected.
row = 'gpu_devctl';
try
    d = gpuDevice(1);
    rng(61, 'twister');
    checks = {};
    try
        checks{end+1} = kf_atleast('device_count', gpuDeviceCount, 1);
        checks{end+1} = kf_atleast('device_table_rows', height(gpuDeviceTable), 1);
        checks{end+1} = kf_count('device_unavailable', double(~parallel.gpu.GPUDevice.isAvailable(1)), 0);
        avail0 = d.AvailableMemory;
        stale = 0;
        for cyc = 1:4
            A = rand(2048, 2048, 'single');
            B = rand(2048, 2048, 'single');
            Ag = gpuArray(A);
            Bg = gpuArray(B);
            for k = 1:20
                Cg = Ag * Bg;
            end
            wait(d);
            checks{end+1} = kf_cmp(sprintf('cycle_matmul_single_%d', cyc), Cg, A * B, kf_tol('single')); %#ok<SAGROW>
            kept = Ag;
            reset(d);
            stale = stale + double(existsOnGPU(kept));
            d = gpuDevice(1);
        end
        checks{end+1} = kf_count('exists_after_reset', stale, 0);
        checks{end+1} = kf_range('avail_ratio_after_reset', d.AvailableMemory / avail0, 0.85, 1.10);
        checks{end+1} = kf_count('gpu_selected_after_reset', double(~(d.DeviceSelected && d.Index == 1)), 0);
        Ag = gpuArray(rand(1024, 1024, 'single'));
        t = gputimeit(@() Ag * Ag);
        checks{end+1} = kf_count('gputimeit_nonpositive', double(~(isfinite(t) && t > 0)), 0);
        checks{end+1} = kf_count('devctl_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('devctl_status', ME);
    end
    doc = struct();
    doc.status = 'done';
    doc.checks = checks;
    kf_write(row, doc);
catch ME
    kf_fail(row, ME);
    rethrow(ME);
end
