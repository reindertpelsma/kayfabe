% dl_layers: the layers and functions deep-learning code is made of, called as dlarray operations with gradients: 1-D, 2-D and 3-D convolution, transposed convolution,
% max and average pooling, batch, group and layer normalisation, self-attention, LSTM and GRU, GELU, softmax cross-entropy and a fully connected layer. Each runs once on the
% CPU and once on the GPU with identical data; the output and one vector holding the gradients with respect to the input and every parameter are compared (double, and one
% convolution in single). Every operation reports its own status, so one unsupported call does not hide the others.
% Falsifier: an output or a gradient vector misses the host's tolerance for its precision, or a GPU operation throws where the CPU one does not.
row = 'dl_layers';
try
    d = gpuDevice(1);
    checks = {};
    ops = {'conv1d', 'conv2d', 'conv2d_s', 'conv3d', 'transpconv2d', 'maxpool2d', 'avgpool2d', 'batchnorm', 'groupnorm', 'layernorm', 'attention', 'lstm', 'gru', 'gelu', ...
        'softmax_ce', 'fullyconnect'};
    fmts = {'SCB', 'SSCB', 'SSCB', 'SSSCB', 'SSCB', 'SSCB', 'SSCB', 'SSCB', 'SSCB', 'CB', 'CBT', 'CBT', 'CBT', 'CB', 'CB', 'CB'};
    xsz = {[64 4 8], [24 24 3 4], [24 24 3 4], [10 10 10 2 2], [12 12 4 2], [16 16 3 2], [16 16 3 2], [8 8 4 6], [8 8 4 3], [12 6], [16 3 7], [4 3 6], [4 3 6], [10 7], [10 6], [12 6]};
    psz = {{[5 4 6], [6 1]}, {[3 3 3 8], [8 1]}, {[3 3 3 8], [8 1]}, {[3 3 3 2 4], [4 1]}, {[3 3 5 4], [5 1]}, {}, {}, {[4 1], [4 1]}, {[4 1], [4 1]}, {[12 1], [12 1]}, {}, ...
        {[20 4], [20 5], [20 1], [5 3], [5 3]}, {[15 4], [15 5], [15 1], [5 3]}, {}, {}, {[7 12], [7 1]}};
    dts = {'double', 'double', 'single', 'double', 'double', 'double', 'double', 'double', 'double', 'double', 'double', 'double', 'double', 'double', 'double', 'double'};
    proof = false;
    for i = 1:numel(ops)
        op = ops{i};
        dt = dts{i};
        try
            rng(100 + i, 'twister');
            x = randn(xsz{i}, dt);
            P = cell(1, numel(psz{i}));
            for q = 1:numel(P)
                P{q} = 0.3 * randn(psz{i}{q}, dt);
            end
            if any(strcmp(op, {'batchnorm', 'groupnorm', 'layernorm'}))
                P{2} = 1 + 0.1 * P{2};
            end
            xc = dlarray(x, fmts{i});
            Pc = cellfun(@dlarray, P, 'UniformOutput', false);
            xg = dlarray(gpuArray(x), fmts{i});
            Pg = cellfun(@(p) dlarray(gpuArray(p)), P, 'UniformOutput', false);
            [yc, gxc, gPc] = dlfeval(@kf_dl_probe, op, xc, Pc);
            [yg, gxg, gPg] = dlfeval(@kf_dl_probe, op, xg, Pg);
            checks{end+1} = kf_cmp(sprintf('%s_y_%s', op, dt), extractdata(yg), gather(extractdata(yc)), kf_tol(dt)); %#ok<SAGROW>
            checks{end+1} = kf_cmp(sprintf('%s_g_%s', op, dt), kf_dl_flat([{gxg}, gPg(:)']), gather(kf_dl_flat([{gxc}, gPc(:)'])), kf_tol(dt)); %#ok<SAGROW>
            checks{end+1} = kf_count(sprintf('%s_status', op), 0, 0); %#ok<SAGROW>
            if ~proof
                checks{end+1} = kf_gpu('layers_env_gpu', extractdata(xg)); %#ok<SAGROW>
                proof = true;
            end
        catch ME
            checks{end+1} = kf_err(sprintf('%s_status', op), ME); %#ok<SAGROW>
        end
    end
    if ~proof
        checks{end+1} = kf_err('layers_env_gpu', MException('kf:layers:noop', 'no operation ran'));
    end
    doc = struct();
    doc.status = 'done';
    doc.checks = checks;
    kf_write(row, doc);
catch ME
    kf_fail(row, ME);
    rethrow(ME);
end
