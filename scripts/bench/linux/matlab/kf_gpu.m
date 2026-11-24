function c = kf_gpu(name, x)
% The operation-on-a-gpuArray proof, taken BEFORE any gather: x must be a gpuArray that lives on the device. A regular array that slipped into a row
% (a gather() of a non-gpuArray returns it unchanged) would otherwise make GPU and CPU agree trivially. The host recomputes the verdict from these
% flags and from its own counters; a guest flag can only add a failure, never override the host's "no GPU work".
c = struct();
c.name = name;
c.kind = 'gpuarray';
c.on_gpu = isa(x, 'gpuArray');
c.exists_on_gpu = c.on_gpu && existsOnGPU(x);
c.underlying = '';
if c.on_gpu
    c.underlying = classUnderlying(x);
end
g = gpuDevice();
c.device_index = double(g.Index);
c.device_name = g.Name;
c.guest_pass = c.on_gpu && c.exists_on_gpu;
end
