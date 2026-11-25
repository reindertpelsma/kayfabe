function t = kf_tol(dt)
% The relative tolerance of a GPU-versus-CPU comparison by precision. The host recomputes the verdict with its own copy of this table.
if strcmp(dt, 'single')
    t = 1e-4;
else
    t = 1e-10;
end
end
