% mem_oversub: allocate gpuArray chunks up to about 85 percent of the memory MATLAB reports free, in 128 MiB steps (bounded loop, at most 512),
% verify every chunk's content, then free them. Nothing here tries to exhaust the device.
row = 'mem_oversub';
try
    d = gpuDevice(1);
    avail0 = d.AvailableMemory;
    chunkBytes = 128 * 2^20;
    nel = chunkBytes / 4;
    target = floor(0.85 * avail0);
    nchunks = min(floor(target / chunkBytes), 512);
    assert(nchunks >= 1, 'kf:nomem');
    chunks = cell(1, nchunks);
    for i = 1:nchunks
        c = gpuArray.ones(nel, 1, 'uint32');
        chunks{i} = c * uint32(i);
        clear c
    end
    wait(d);
    availAlloc = d.AvailableMemory;
    mism = zeros(1, nchunks);
    for i = 1:nchunks
        mism(i) = gather(sum(chunks{i} ~= uint32(i)));
    end
    checks = {};
    checks{end+1} = kf_count('content_mismatches', sum(mism), 0);
    for i = unique([1, ceil(nchunks / 2), nchunks])
        s = gather(chunks{i}([1, nel / 2, nel]));
        checks{end+1} = kf_cmp(sprintf('sample_chunk_%d', i), s, uint32([i; i; i]), 0); %#ok<SAGROW>
    end
    clear chunks
    wait(d);
    availEnd = d.AvailableMemory;
    mem = struct();
    mem.total = d.TotalMemory;
    mem.avail_start = avail0;
    mem.avail_after_alloc = availAlloc;
    mem.avail_end = availEnd;
    mem.chunk_bytes = chunkBytes;
    mem.chunks = nchunks;
    mem.allocated_bytes = nchunks * chunkBytes;
    doc = struct();
    doc.status = 'done';
    doc.memory = mem;
    doc.checks = checks;
    kf_write(row, doc);
catch ME
    kf_fail(row, ME);
    rethrow(ME);
end
