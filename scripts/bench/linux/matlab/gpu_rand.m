% gpu_rand: the GPU random number generators (Philox4x32-10, Threefry4x64-20, CombRecursive) through gpurng. Per generator: re-seeding reproduces the stream exactly,
% a different seed gives a different stream, and 2^24 uniform and normal draws have the right mean, variance, uniformity and lag-1 correlation. Also randi bounds and
% randperm. The oracle is the stream itself (exact reproducibility) and the host's closed intervals (kf_range; at least ten standard deviations wide).
% Falsifier: a re-seeded stream differs in one element, a statistic leaves the host's interval, or a generator cannot be selected.
row = 'gpu_rand';
try
    d = gpuDevice(1);
    checks = {};
    gens = {'Philox4x32-10', 'Threefry4x64-20', 'CombRecursive'};
    tags = {'philox', 'threefry', 'combrecursive'};
    N = 2 ^ 24;
    try
        for g = 1:3
            tg = tags{g};
            gpurng(7, gens{g});
            a = rand(N, 1, 'single', 'gpuArray');
            gpurng(7, gens{g});
            b = rand(N, 1, 'single', 'gpuArray');
            checks{end+1} = kf_count(sprintf('repro_mismatch_%s', tg), sum(a ~= b), 0); %#ok<SAGROW>
            gpurng(8, gens{g});
            c = rand(N, 1, 'single', 'gpuArray');
            checks{end+1} = kf_range(sprintf('rand_diffseed_equal_%s', tg), mean(double(a == c)), 0, 0.001); %#ok<SAGROW>
            if g == 1
                checks{end+1} = kf_gpu('rand_is_gpuarray', a); %#ok<SAGROW>
            end
            gpurng(11, gens{g});
            u = rand(N, 1, 'double', 'gpuArray');
            checks{end+1} = kf_range(sprintf('unif_mean_%s', tg), mean(u), 0.499, 0.501); %#ok<SAGROW>
            checks{end+1} = kf_range(sprintf('unif_var_%s', tg), var(u), 0.0813, 0.0853); %#ok<SAGROW>
            h = histcounts(u, 0:0.1:1) / N;
            checks{end+1} = kf_range(sprintf('unif_hist_dev_%s', tg), max(abs(h - 0.1)), 0, 0.003); %#ok<SAGROW>
            c0 = u - 0.5;
            checks{end+1} = kf_range(sprintf('unif_lag1_%s', tg), sum(c0(1:end-1) .* c0(2:end)) / sum(c0 .^ 2), -0.002, 0.002); %#ok<SAGROW>
            z = randn(N, 1, 'double', 'gpuArray');
            checks{end+1} = kf_range(sprintf('norm_mean_%s', tg), mean(z), -0.002, 0.002); %#ok<SAGROW>
            checks{end+1} = kf_range(sprintf('norm_var_%s', tg), var(z), 0.99, 1.01); %#ok<SAGROW>
        end
        gpurng(3, 'Philox4x32-10');
        s = zeros(1, 1, 'gpuArray');
        for k = 1:100
            s = s + sum(randn(N, 1, 'single', 'gpuArray'));
        end
        wait(d);
        r = randi(10, N, 1, 'gpuArray');
        checks{end+1} = kf_range('randi_min', min(r), 1, 1);
        checks{end+1} = kf_range('randi_max', max(r), 10, 10);
        checks{end+1} = kf_count('rand_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('rand_status', ME);
    end
    try
        p = gather(sort(randperm(gpuArray(1000000))));
        checks{end+1} = kf_count('randperm_not_permutation', double(~isequal(p(:), (1:1000000)')), 0);
    catch ME
        checks{end+1} = kf_err('randperm_not_permutation', ME);
    end
    doc = struct();
    doc.status = 'done';
    doc.checks = checks;
    kf_write(row, doc);
catch ME
    kf_fail(row, ME);
    rethrow(ME);
end
