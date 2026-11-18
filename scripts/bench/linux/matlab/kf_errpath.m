function c = kf_errpath(name, fcpu, fgpu)
% An error path: the same failing call on the CPU (fcpu, may be empty when no CPU analogue exists) and on the GPU (fgpu). Records the identifiers only; the host judges the class
% (the first two components of the identifier) against its own table and, where a CPU analogue exists, against the CPU's.
c = struct();
c.name = name;
c.kind = 'errid';
c.cpu_id = '';
c.gpu_id = '';
c.raised = false;
if ~isempty(fcpu)
    try
        fcpu();
    catch ME
        c.cpu_id = ME.identifier;
    end
end
try
    fgpu();
catch ME
    c.raised = true;
    c.gpu_id = ME.identifier;
end
c.guest_pass = c.raised;
end
