function s = kf_enum_train(cfg)
% The cross product the product lists for training: every solver by every execution environment of trainingOptions, one tiny trainnet run each (3 epochs). The CPU run of the same
% solver is the oracle: a cell passes when its final loss agrees with it (10 percent plus 2e-3, the host's pair rule). Errors are counted separately: an error in cpu, gpu or auto
% is unexpected; the multi-GPU and parallel environments may refuse on one GPU, and their identifiers are recorded in the names list.
s = struct('cells', 0, 'pass', 0, 'diff', 0, 'unexpected_error', 0, 'refused', 0, 'names_diff', {{}}, 'names_unexpected_error', {{}}, 'names_refused', {{}});
rng(31, 'twister');
X = randn(256, 4, 'single');
Y = tanh(X * [1; -1; 0.5; 2]);
layers = [featureInputLayer(4); fullyConnectedLayer(8); reluLayer; fullyConnectedLayer(1)];
for a = 1:numel(cfg.solvers)
    sol = cfg.solvers{a};
    ref = NaN;
    for b = 1:numel(cfg.environments)
        envn = cfg.environments{b};
        label = [sol '/' envn];
        s.cells = s.cells + 1;
        try
            rng(41, 'twister');
            if strcmp(sol, 'lbfgs')
                opts = trainingOptions(sol, 'ExecutionEnvironment', envn, 'MaxIterations', 30, 'Plots', 'none', 'Verbose', false);        % full-batch solver: no MiniBatchSize, MaxEpochs or Shuffle
            else
                opts = trainingOptions(sol, 'ExecutionEnvironment', envn, 'MaxEpochs', 3, 'MiniBatchSize', 64, 'Shuffle', 'never', 'Plots', 'none', 'Verbose', false);
            end
            net = trainnet(X, Y, layers, 'mse', opts);
            P = minibatchpredict(net, X, 'ExecutionEnvironment', 'cpu');
            loss = mean((double(P(:)) - double(Y(:))) .^ 2);
            if strcmp(envn, 'cpu')
                ref = loss;
                s.pass = s.pass + 1;
            elseif isnan(ref) || abs(loss - ref) > 2e-3 + 0.10 * max(abs(loss), abs(ref))
                s.diff = s.diff + 1;
                s.names_diff{end+1} = label;
            else
                s.pass = s.pass + 1;
            end
        catch ME
            if any(strcmp(envn, {'cpu', 'gpu', 'auto'}))
                s.unexpected_error = s.unexpected_error + 1;
                s.names_unexpected_error{end+1} = [label ':' ME.identifier];
            else
                s.refused = s.refused + 1;
                s.names_refused{end+1} = [label ':' ME.identifier];
            end
        end
    end
    try
        delete(gcp('nocreate'));
    catch
    end
end
end
