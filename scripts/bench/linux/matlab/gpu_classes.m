% gpu_classes: data classes and primitives that are not floating-point numerics: saturating int8 arithmetic, int32 scan and mod, uint32 and uint64 bit operations,
% logical masks, find, sort (values and a permutation), histcounts, accumarray, unique, ismember, complex arithmetic in both precisions, and a class round trip.
% Everything integer-valued is compared exactly with the CPU. Falsifier: any exact check differs in one element, or a complex result misses the host's tolerance.
row = 'gpu_classes';
try
    d = gpuDevice(1);
    rng(41, 'twister');
    checks = {};
    n = 4000000;
    try
        a = randi([-128 127], n, 1, 'int8');
        b = randi([-128 127], n, 1, 'int8');
        ga = gpuArray(a);
        gb = gpuArray(b);
        checks{end+1} = kf_cmp('exact_int8_saturate', (ga + gb) + (ga .* gb) - gb, (a + b) + (a .* b) - b, 0);
        x = randi([-100 100], n, 1, 'int32');
        gx = gpuArray(x);
        checks{end+1} = kf_cmp('exact_int32_cumsum', cumsum(gx), cumsum(x), 0);
        checks{end+1} = kf_cmp('exact_int32_mod', [mod(gx, int32(97)); rem(gx, int32(97))], [mod(x, int32(97)); rem(x, int32(97))], 0);
        u = randi([0 double(intmax('uint32'))], n, 1, 'uint32');
        v = randi([0 double(intmax('uint32'))], n, 1, 'uint32');
        gu = gpuArray(u);
        gv = gpuArray(v);
        checks{end+1} = kf_cmp('exact_uint32_bitops', bitxor(bitand(gu, gv), bitor(bitshift(gu, 3), bitshift(gv, -5))), ...
            bitxor(bitand(u, v), bitor(bitshift(u, 3), bitshift(v, -5))), 0);
        w = uint64(2) ^ 63 + uint64(randi(1000000000, n, 1));
        gw = gpuArray(w);
        checks{end+1} = kf_cmp('exact_uint64_large', bitxor(gw, bitshift(gw, -13)) + uint64(12345), bitxor(w, bitshift(w, -13)) + uint64(12345), 0);
        checks{end+1} = kf_count('int_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('int_status', ME);
    end
    try
        X = rand(n, 1);
        Y = rand(n, 1);
        gX = gpuArray(X);
        gY = gpuArray(Y);
        m1 = X > 0.2;
        m2 = Y < 0.7;
        g1 = gX > 0.2;
        g2 = gY < 0.7;
        checks{end+1} = kf_cmp('exact_logical_ops', (g1 & g2) | xor(g1, ~g2), (m1 & m2) | xor(m1, ~m2), 0);
        checks{end+1} = kf_cmp('exact_find_indices', find(g1 & g2), find(m1 & m2), 0);
        checks{end+1} = kf_count('logical_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('logical_status', ME);
    end
    try
        gs = sort(gX);
        checks{end+1} = kf_cmp('exact_sort_values', gs, sort(X), 0);
        p = randperm(n)';
        [~, gi] = sort(gpuArray(p));
        [~, ci] = sort(p);
        checks{end+1} = kf_cmp('exact_sort_perm_indices', gi, ci, 0);
        checks{end+1} = kf_cmp('cumsum_double', cumsum(gX), cumsum(X), kf_tol('double'));
        edges = 0:0.01:1;
        checks{end+1} = kf_cmp('exact_histcounts', histcounts(gX, edges), histcounts(X, edges), 0);
        subs = randi(1000, n, 1);
        vals = randi(10, n, 1);
        checks{end+1} = kf_cmp('exact_accumarray', accumarray(gpuArray(subs), gpuArray(vals)), accumarray(subs, vals), 0);
        q = randi(5000, n, 1);
        checks{end+1} = kf_cmp('exact_unique', unique(gpuArray(q)), unique(q), 0);
        r = randi(10000, n, 1);
        checks{end+1} = kf_cmp('exact_ismember', ismember(gpuArray(r), 1:2:10000), ismember(r, 1:2:10000), 0);
        checks{end+1} = kf_count('scan_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('scan_status', ME);
    end
    try
        types = {'single', 'double'};
        for t = 1:2
            dt = types{t};
            z = complex(randn(1000000, 1, dt), randn(1000000, 1, dt));
            f = @(z) z .* exp(1i * imag(z)) + conj(z) ./ (1 + abs(z)) + z .^ 2 ./ (2 + real(z) .^ 2);
            checks{end+1} = kf_cmp(sprintf('complex_arith_%s', dt), f(gpuArray(z)), f(z), kf_tol(dt)); %#ok<SAGROW>
        end
        checks{end+1} = kf_count('complex_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('complex_status', ME);
    end
    classes = {'single', 'double', 'int8', 'int16', 'int32', 'int64', 'uint8', 'uint16', 'uint32', 'uint64', 'logical'};
    bad = 0;
    for c = 1:numel(classes)
        x = cast(randi(100, 1000, 1), classes{c});
        y = gather(gpuArray(x));
        bad = bad + double(~isequal(y, x) || ~strcmp(class(y), classes{c}));
    end
    checks{end+1} = kf_count('roundtrip_mismatches', bad, 0);
    doc = struct();
    doc.status = 'done';
    doc.checks = checks;
    kf_write(row, doc);
catch ME
    kf_fail(row, ME);
    rethrow(ME);
end
