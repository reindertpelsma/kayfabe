% dl_train: a tiny fully-connected network trained with the Deep Learning Toolbox on synthetic data, once on the GPU and once on the CPU with
% the same data, seed and options. The loss must fall, and the two final losses must agree within the host's stated tolerance.
row = 'dl_train';
try
    d = gpuDevice(1);
    nfeat = 16;
    nobs = 8192;
    batch = 256;
    epochs = 30;
    rng(7, 'twister');
    X = randn(nobs, nfeat, 'single');
    w = randn(nfeat, 1, 'single');
    Y = tanh(X * w) + 0.05 * randn(nobs, 1, 'single');
    layers = [featureInputLayer(nfeat); fullyConnectedLayer(64); reluLayer; fullyConnectedLayer(64); reluLayer; fullyConnectedLayer(1)];
    envs = {'gpu', 'cpu'};
    curve = cell(1, 2);
    final = zeros(1, 2);
    for e = 1:2
        rng(11, 'twister');
        opts = trainingOptions('adam', 'ExecutionEnvironment', envs{e}, 'MaxEpochs', epochs, 'MiniBatchSize', batch, ...
            'InitialLearnRate', 0.01, 'Shuffle', 'never', 'Plots', 'none', 'Verbose', false);
        [net, info] = trainnet(X, Y, layers, 'mse', opts);
        h = double(info.TrainingHistory.Loss);
        h = h(:);
        per = nobs / batch;
        ne = floor(numel(h) / per);
        ep = mean(reshape(h(1:ne * per), per, ne), 1);
        curve{e} = ep;
        P = minibatchpredict(net, X, 'ExecutionEnvironment', 'cpu');
        final(e) = mean((double(P(:)) - double(Y(:))) .^ 2);
    end
    % the training ran with ExecutionEnvironment 'gpu' (an error without a GPU, never a CPU fallback); the proof item records the device and a gpuArray
    % made from the same data, and the host's census and nvidia-smi are the authority on the training itself
    c0 = kf_gpu('train_env_gpu', gpuArray(X(1:8, :)));
    c0.env_requested = envs{1};
    c1 = struct();
    c1.name = 'train_loss_curve';
    c1.kind = 'series';
    c1.gpu = curve{1};
    c1.cpu = curve{2};
    c2 = struct();
    c2.name = 'final_mse';
    c2.kind = 'pair';
    c2.gpu = final(1);
    c2.cpu = final(2);
    doc = struct();
    doc.status = 'done';
    doc.epochs = epochs;
    doc.checks = {c0, c1, c2};
    kf_write(row, doc);
catch ME
    kf_fail(row, ME);
    rethrow(ME);
end
