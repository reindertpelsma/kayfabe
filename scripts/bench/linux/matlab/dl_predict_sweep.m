% dl_predict_sweep: inference and training as a user runs them: predict over a small CNN with MiniBatchSize 1, 7, 64, 1000 and 2048 (including a size larger than the data),
% against the CPU and against each other (the result must not depend on the batch size), the minibatchpredict API on the GPU against the CPU, a trainnet run with
% checkpoints on the GPU and on the CPU, loading the last checkpoint and predicting with it, and resuming training from it.
% Falsifier: a batch size changes the answer beyond the single tolerance, the loss does not fall to half on both devices, the checkpoint network predicts differently from the
% network that trainnet returned, or resumed training makes the loss worse.
row = 'dl_predict_sweep';
try
    d = gpuDevice(1);
    checks = {};
    N = 2048;
    rng(5, 'twister');
    Xs = randn(16, 16, 1, N, 'single');
    scores = reshape(Xs, 256, N)' * randn(256, 10, 'single');
    [~, lab] = max(scores, [], 2);
    T = categorical(lab);
    layers = [imageInputLayer([16 16 1], 'Normalization', 'none'); convolution2dLayer(3, 8, 'Padding', 'same'); batchNormalizationLayer; reluLayer; ...
        maxPooling2dLayer(2, 'Stride', 2); fullyConnectedLayer(10); softmaxLayer];
    try
        rng(6, 'twister');
        net = dlnetwork(layers);
        ref = gather(kf_dl_predict(net, Xs, 64, false));
        P7 = [];
        P1000 = [];
        for mb = [1 7 64 1000 2048]
            Pg = kf_dl_predict(net, Xs, mb, true);
            checks{end+1} = kf_cmp(sprintf('predict_mb_single_%d', mb), Pg, ref, kf_tol('single')); %#ok<SAGROW>
            if mb == 7
                P7 = Pg;
            elseif mb == 1000
                P1000 = Pg;
                api = minibatchpredict(net, Xs, 'MiniBatchSize', 1000, 'ExecutionEnvironment', 'gpu');
                apic = minibatchpredict(net, Xs, 'MiniBatchSize', 1000, 'ExecutionEnvironment', 'cpu');
                checks{end+1} = kf_resid('predict_api_resid_single_1000', norm(double(api(:)) - double(apic(:))) / max(norm(double(apic(:))), 1e-300), 1e-4); %#ok<SAGROW>
            end
        end
        checks{end+1} = kf_cmp('predict_invariance_single', P7, gather(P1000), kf_tol('single'));
        checks{end+1} = kf_gpu('predict_env_gpu', P7);
        checks{end+1} = kf_count('sweep_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('sweep_status', ME);
    end
    try
        ck = fullfile(kf_outdir(), 'ckpt');
        if ~isfolder(ck)
            mkdir(ck);
        end
        delete(fullfile(ck, '*.mat'));
        epochs = 10;
        batch = 128;
        curve = cell(1, 2);
        envs = {'gpu', 'cpu'};
        for e = 1:2
            rng(8, 'twister');
            opts = {'adam', 'ExecutionEnvironment', envs{e}, 'MaxEpochs', epochs, 'MiniBatchSize', batch, 'InitialLearnRate', 0.01, 'Shuffle', 'never', 'Plots', 'none', ...
                'Verbose', false};
            if e == 1
                opts = [opts, {'CheckpointPath', ck, 'CheckpointFrequency', 1}];
            end
            [tnet, info] = trainnet(Xs, T, layers, 'crossentropy', trainingOptions(opts{:}));
            h = double(info.TrainingHistory.Loss);
            per = N / batch;
            ne = floor(numel(h) / per);
            curve{e} = mean(reshape(h(1:ne * per), per, ne), 1);
            if e == 1
                final = tnet;
            end
        end
        c1 = struct();
        c1.name = 'train_loss_curve';
        c1.kind = 'series';
        c1.gpu = curve{1};
        c1.cpu = curve{2};
        checks{end+1} = c1;
        files = dir(fullfile(ck, 'net_checkpoint__*.mat'));
        checks{end+1} = kf_atleast('checkpoint_files', numel(files), 1);
        [~, last] = max([files.datenum]);
        s = load(fullfile(ck, files(last).name));
        Pck = kf_dl_predict(s.net, Xs(:, :, :, 1:512), 128, true);
        Pfin = gather(kf_dl_predict(final, Xs(:, :, :, 1:512), 128, false));
        checks{end+1} = kf_cmp('checkpoint_vs_final_single', Pck, Pfin, kf_tol('single'));
        checks{end+1} = kf_count('ckpt_status', 0, 0);
        try
            rng(9, 'twister');
            opts2 = trainingOptions('adam', 'ExecutionEnvironment', 'gpu', 'MaxEpochs', 4, 'MiniBatchSize', batch, 'InitialLearnRate', 0.005, 'Shuffle', 'never', 'Plots', 'none', ...
                'Verbose', false);
            [~, info2] = trainnet(Xs, T, s.net, 'crossentropy', opts2);
            h2 = double(info2.TrainingHistory.Loss);
            ne2 = floor(numel(h2) / per);
            e2 = mean(reshape(h2(1:ne2 * per), per, ne2), 1);
            checks{end+1} = kf_count('resume_loss_regress', double(e2(end) > 1.05 * e2(1)), 0);
            checks{end+1} = kf_count('resume_status', 0, 0);
        catch ME
            checks{end+1} = kf_err('resume_status', ME);
        end
    catch ME
        checks{end+1} = kf_err('ckpt_status', ME);
    end
    doc = struct();
    doc.status = 'done';
    doc.epochs = 10;
    doc.checks = checks;
    kf_write(row, doc);
catch ME
    kf_fail(row, ME);
    rethrow(ME);
end
