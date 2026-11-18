% dl_custom_loop: deep learning as people write it by hand: a dlnetwork trained with dlfeval, dlgradient and adamupdate on the GPU and on the CPU from the same initial
% weights and data (the loss must fall and the final losses agree), an analytic gradient check, five steps of each of adamupdate, sgdmupdate and rmspropupdate compared
% parameter by parameter, and dlaccelerate (the accelerated function against the plain one, with at least one cache hit).
% Falsifier: the loss does not fall to half on either device, the final losses differ beyond the host's pair tolerance, a gradient or an updated parameter vector misses
% the single tolerance, or dlaccelerate returns different numbers or never hits its cache.
row = 'dl_custom_loop';
try
    d = gpuDevice(1);
    checks = {};
    nfeat = 16;
    nobs = 4096;
    iters = 100;
    rng(7, 'twister');
    X = randn(nfeat, nobs, 'single');
    w = randn(nfeat, 1, 'single');
    Y = tanh(w' * X) + 0.05 * randn(1, nobs, 'single');
    layers = [featureInputLayer(nfeat); fullyConnectedLayer(32); tanhLayer; fullyConnectedLayer(1)];
    rng(11, 'twister');
    dnet0 = dlnetwork(layers);
    dXg = dlarray(gpuArray(X), 'CB');
    dYg = dlarray(gpuArray(Y), 'CB');
    dXc = dlarray(X, 'CB');
    dYc = dlarray(Y, 'CB');
    try
        curve = zeros(2, iters);
        for e = 1:2
            dnet = dnet0;
            if e == 1
                dnet = dlupdate(@gpuArray, dnet);
                dX = dXg;
                dY = dYg;
            else
                dX = dXc;
                dY = dYc;
            end
            avg = [];
            sqavg = [];
            for it = 1:iters
                [loss, grads] = dlfeval(@kf_dl_loss, dnet, dX, dY);
                [dnet, avg, sqavg] = adamupdate(dnet, grads, avg, sqavg, it, 0.02);
                curve(e, it) = double(gather(extractdata(loss)));
            end
        end
        checks{end+1} = kf_gpu('custom_env_gpu', extractdata(dXg));
        c1 = struct();
        c1.name = 'custom_loss_curve';
        c1.kind = 'series';
        c1.gpu = curve(1, :);
        c1.cpu = curve(2, :);
        checks{end+1} = c1;
        c2 = struct();
        c2.name = 'custom_final_mse';
        c2.kind = 'pair';
        c2.gpu = curve(1, end);
        c2.cpu = curve(2, end);
        checks{end+1} = c2;
        xg = dlarray(gpuArray(randn(1000, 1)));
        xc = gather(extractdata(xg));
        gg = dlfeval(@(x) dlgradient(sum(x .^ 2 .* sin(x), 'all'), x), xg);
        checks{end+1} = kf_cmp('gradcheck_double', extractdata(gg), 2 * xc .* sin(xc) + xc .^ 2 .* cos(xc), kf_tol('double'));
        checks{end+1} = kf_count('loop_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('loop_status', ME);
    end
    try
        names = {'adamupdate', 'sgdmupdate', 'rmspropupdate'};
        for o = 1:3
            fin = cell(1, 2);
            for e = 1:2
                dnet = dnet0;
                if e == 1
                    dnet = dlupdate(@gpuArray, dnet);
                    dX = dXg;
                    dY = dYg;
                else
                    dX = dXc;
                    dY = dYc;
                end
                s1 = [];
                s2 = [];
                for it = 1:5
                    [~, grads] = dlfeval(@kf_dl_loss, dnet, dX, dY);
                    switch o
                        case 1
                            [dnet, s1, s2] = adamupdate(dnet, grads, s1, s2, it, 0.01);
                        case 2
                            [dnet, s1] = sgdmupdate(dnet, grads, s1, 0.05, 0.9);
                        case 3
                            [dnet, s1] = rmspropupdate(dnet, grads, s1, 0.01);
                    end
                end
                fin{e} = kf_dl_flat(dnet.Learnables);
            end
            checks{end+1} = kf_cmp(sprintf('%s_params_single', names{o}), fin{1}, gather(fin{2}), kf_tol('single')); %#ok<SAGROW>
        end
        checks{end+1} = kf_count('optim_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('optim_status', ME);
    end
    try
        dnet = dlupdate(@gpuArray, dnet0);
        accfun = dlaccelerate(@kf_dl_loss);
        clearCache(accfun);
        for k = 1:4
            [la, ga] = dlfeval(accfun, dnet, dXg, dYg);
        end
        [lp, gp] = dlfeval(@kf_dl_loss, dnet, dXg, dYg);
        checks{end+1} = kf_cmp('accel_vs_plain_single', kf_dl_flat(ga), gather(kf_dl_flat(gp)), kf_tol('single'));
        checks{end+1} = kf_atleast('accel_cache_hits', double(accfun.HitRate > 0), 1);
        checks{end+1} = kf_count('accel_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('accel_status', ME);
    end
    doc = struct();
    doc.status = 'done';
    doc.checks = checks;
    kf_write(row, doc);
catch ME
    kf_fail(row, ME);
    rethrow(ME);
end
