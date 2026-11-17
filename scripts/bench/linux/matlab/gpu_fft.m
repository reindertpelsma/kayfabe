% gpu_fft: fft/ifft round trips on gpuArray (power-of-two, composite and prime lengths, 2-D), compared with the CPU, then a fixed-count sustained phase.
row = 'gpu_fft';
try
    d = gpuDevice(1);
    rng(12345, 'twister');
    checks = {};
    types = {'single', 'double'};
    for t = 1:2
        dt = types{t};
        for n = [65536 1048576 30030 10007]
            x = complex(rand(n, 1, dt), rand(n, 1, dt));
            xg = gpuArray(x);
            Xg = fft(xg);
            checks{end+1} = kf_cmp(sprintf('fft_%s_%d', dt, n), Xg, fft(x), kf_tol(dt)); %#ok<SAGROW>
            checks{end+1} = kf_cmp(sprintf('roundtrip_%s_%d', dt, n), ifft(Xg), x, kf_tol(dt)); %#ok<SAGROW>
        end
        x2 = complex(rand(1024, 1024, dt), rand(1024, 1024, dt));
        X2g = fft2(gpuArray(x2));
        checks{end+1} = kf_cmp(sprintf('fft2_%s_1024', dt), X2g, fft2(x2), kf_tol(dt)); %#ok<SAGROW>
        checks{end+1} = kf_cmp(sprintf('roundtrip2_%s_1024', dt), ifft2(X2g), x2, kf_tol(dt)); %#ok<SAGROW>
    end
    x2 = complex(rand(2048, 2048, 'single'), rand(2048, 2048, 'single'));
    x2g = gpuArray(x2);
    for k = 1:200
        Y = fft2(x2g);
    end
    wait(d);
    checks{end+1} = kf_cmp('fft2_single_2048_after_200', Y, fft2(x2), kf_tol('single'));
    doc = struct();
    doc.status = 'done';
    doc.checks = checks;
    kf_write(row, doc);
catch ME
    kf_fail(row, ME);
    rethrow(ME);
end
