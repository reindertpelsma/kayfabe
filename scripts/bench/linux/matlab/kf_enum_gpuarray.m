function s = kf_enum_gpuarray(cfg)
% A function-by-function sweep of the methods of gpuArray (what the product itself lists): each unary method is called once on a standard 8-by-8 matrix on the GPU and
% on the CPU. Classes: pass (same size, same class, equal within tolerance), diff, gpu_only_error (the CPU call works and the GPU call throws), both_error (needs other
% arguments: not testable by a unary sweep), nonnumeric (output not compared). Returns counts and the bounded name lists.
names = methods('gpuArray');
names = names(:)';
skipx = cfg.gpuarray_skip_exact;
skipp = cfg.gpuarray_skip_prefix;
rng(1, 'twister');
xc = 0.1 + 0.8 * rand(8, 8);
xg = gpuArray(xc);
s = struct('tested', 0, 'pass', 0, 'diff', 0, 'gpu_only_error', 0, 'both_error', 0, 'nonnumeric', 0, 'skipped', 0, 'names_diff', {{}}, 'names_gpu_only_error', {{}});
for i = 1:numel(names)
    n = names{i};
    if any(strcmp(n, skipx)) || any(startsWith(n, skipp)) || ~isvarname(n)
        s.skipped = s.skipped + 1;
        continue
    end
    cerr = false;
    gerr = false;
    yc = [];
    yg = [];
    try
        yc = feval(n, xc);
    catch
        cerr = true;
    end
    try
        yg = feval(n, xg);
        wait(gpuDevice);
    catch
        gerr = true;
    end
    if cerr && gerr
        s.both_error = s.both_error + 1;
        continue
    end
    s.tested = s.tested + 1;
    if gerr
        s.gpu_only_error = s.gpu_only_error + 1;
        s.names_gpu_only_error{end+1} = n;
        continue
    end
    if cerr || ~(isnumeric(yc) || islogical(yc))
        s.nonnumeric = s.nonnumeric + 1;
        continue
    end
    ok = false;
    try
        g = gather(yg);
        ok = isequal(size(g), size(yc)) && strcmp(class(g), class(yc)) && (isequaln(g, yc) || ...
            norm(double(g(:)) - double(yc(:)), 'fro') <= 1e-8 * max(norm(double(yc(:)), 'fro'), 1) || all(isnan(g(:)) == isnan(yc(:))) && ...
            norm(double(g(~isnan(g))) - double(yc(~isnan(yc))), 'fro') <= 1e-8 * max(norm(double(yc(~isnan(yc))), 'fro'), 1));
    catch
        ok = false;
    end
    if ok
        s.pass = s.pass + 1;
    else
        s.diff = s.diff + 1;
        s.names_diff{end+1} = n;
    end
end
end
