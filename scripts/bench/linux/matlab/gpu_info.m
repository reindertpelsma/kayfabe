% gpu_info: preamble row. Records the GPU device facts MATLAB reports and one tiny GPU-versus-CPU sum. No account, licence or path output.
row = 'gpu_info';
try
    d = gpuDevice(1);
    info = struct();
    info.name = d.Name;
    info.compute_capability = d.ComputeCapability;
    info.driver_version = d.DriverVersion;
    info.toolkit_version = d.ToolkitVersion;
    info.total_memory = d.TotalMemory;
    info.available_memory = d.AvailableMemory;
    info.multiprocessors = d.MultiprocessorCount;
    info.device_count = gpuDeviceCount;
    info.matlab_release = version('-release');
    info.can_use_gpu = canUseGPU;
    x = ones(1e6, 1, 'double');
    checks = {};
    checks{end+1} = kf_cmp('sum_ones_1e6', sum(gpuArray(x)), sum(x), 0);
    doc = struct();
    doc.status = 'done';
    doc.info = info;
    doc.checks = checks;
    kf_write(row, doc);
catch ME
    kf_fail(row, ME);
    rethrow(ME);
end
