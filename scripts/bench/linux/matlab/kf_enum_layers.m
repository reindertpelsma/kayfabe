function s = kf_enum_layers(cfg)
% The layer constructors the product lists (data file), each put behind an input layer in a dlnetwork, initialised once on the CPU with a fixed seed, and evaluated with predict
% on the CPU and on the GPU with identical weights and data. Classes as in kf_enum_gpuarray: pass, diff, gpu_only_error, both_error (the constructor or the CPU evaluation fails: our
% config), counts and the bounded name lists.
s = struct('tested', 0, 'pass', 0, 'diff', 0, 'gpu_only_error', 0, 'both_error', 0, 'names_diff', {{}}, 'names_gpu_only_error', {{}}, 'names_both_error', {{}});
for i = 1:numel(cfg.layers)
    item = cfg.layers(i);
    label = regexprep(item.c, '\(.*', '');
    try
        rng(200 + i, 'twister');
        switch item.in
            case 'features'
                inl = featureInputLayer(8);
                x = randn(8, 6);
                fmt = 'CB';
            case 'image'
                inl = imageInputLayer([8 8 3], 'Normalization', 'none');
                x = randn(8, 8, 3, 4);
                fmt = 'SSCB';
            case 'image12'
                inl = imageInputLayer([8 8 4], 'Normalization', 'none');
                x = randn(8, 8, 4, 4);
                fmt = 'SSCB';
            otherwise
                inl = sequenceInputLayer(8);
                x = randn(8, 4, 6);
                fmt = 'CBT';
        end
        mk = str2func(['@() ' item.c]);
        net = dlnetwork([inl; mk()]);
        xs = single(x);
        yc = predict(net, dlarray(xs, fmt));
    catch
        s.both_error = s.both_error + 1;
        s.names_both_error{end+1} = label;
        continue
    end
    s.tested = s.tested + 1;
    try
        ng = dlupdate(@gpuArray, net);
        yg = predict(ng, dlarray(gpuArray(xs), fmt));
        a = gather(extractdata(yg));
        b = extractdata(yc);
        if isequal(size(a), size(b)) && norm(double(a(:)) - double(b(:))) <= 1e-4 * max(norm(double(b(:))), 1)
            s.pass = s.pass + 1;
        else
            s.diff = s.diff + 1;
            s.names_diff{end+1} = label;
        end
    catch
        s.gpu_only_error = s.gpu_only_error + 1;
        s.names_gpu_only_error{end+1} = label;
    end
end
end
