% First kf3 GPU test of image C: gpuDevice, matmul, FFT, Deep Learning Toolbox GPU op, each cross-checked against the CPU.
% Prints PASS/FAIL lines; no guest settings, logs or account data are read.
function matlab_gpu_test
t0 = tic;
fprintf('KFM release=%s\n', version('-release'));
g = gpuDevice;
fprintf('KFM gpu_name=%s cc=%s drv=%.2f toolkit=%.2f totalmem_gb=%.2f\n', g.Name, g.ComputeCapability, g.DriverVersion, g.ToolkitVersion, g.TotalMemory/2^30);
% 1. the requested command
a = gpuArray.rand(4000); b = a*a; wait(g);
fprintf('KFM matmul4000_sum_positive=%d\n', gather(sum(b(:)))>0);
% 2. matmul against the CPU
rng(1); A = rand(2000); B = rand(2000);
tc = tic; C = A*B; tcpu = toc(tc);
tg = tic; Cg = gather(gpuArray(A)*gpuArray(B)); tgpu = toc(tg);
err = max(abs(Cg(:)-C(:)))/max(abs(C(:)));
fprintf('KFM matmul2000_relerr=%.3g cpu_s=%.3f gpu_s=%.3f %s\n', err, tcpu, tgpu, pf(err<1e-10));
% 3. FFT
X = rand(1024,1024) + 1i*rand(1024,1024);
Yg = gather(fft2(gpuArray(X))); Yc = fft2(X);
err = max(abs(Yg(:)-Yc(:)))/max(abs(Yc(:)));
Z = gather(ifft2(fft2(gpuArray(X))));
err2 = max(abs(Z(:)-X(:)));
fprintf('KFM fft2_relerr=%.3g roundtrip_maxerr=%.3g %s\n', err, err2, pf(err<1e-10 && err2<1e-10));
% 4. single precision elementwise and a reduction
s = single(rand(1e6,1)); sg = gather(sum(sin(gpuArray(s)).^2)); sc = sum(sin(s).^2);
fprintf('KFM sin2_sum_relerr=%.3g %s\n', abs(sg-sc)/abs(sc), pf(abs(sg-sc)/abs(sc)<1e-3));
% 5. Deep Learning Toolbox, if installed
if license('test','Neural_Network_Toolbox') && exist('dlarray','file')
    try
        rng(2); x = single(rand(8,8,1,4)); w = single(rand(3,3,1,2)); bb = single(zeros(2,1));
        yg = extractdata(gather(dlconv(dlarray(gpuArray(x),'SSCB'), gpuArray(w), gpuArray(bb))));
        yc = extractdata(dlconv(dlarray(x,'SSCB'), w, bb));
        e = max(abs(yg(:)-yc(:)))/max(abs(yc(:)));
        W = single(rand(5,16)); xv = single(rand(16,4));
        fg = extractdata(gather(fullyconnect(dlarray(gpuArray(xv),'CB'), gpuArray(W), single(zeros(5,1)))));
        fc = extractdata(fullyconnect(dlarray(xv,'CB'), W, single(zeros(5,1))));
        e2 = max(abs(fg(:)-fc(:)))/max(abs(fc(:)));
        fprintf('KFM dl_dlconv_relerr=%.3g fullyconnect_relerr=%.3g %s\n', e, e2, pf(e<1e-4 && e2<1e-4));
    catch ex
        fprintf('KFM dl_error=%s\n', strtok(ex.message, newline));
    end
else
    fprintf('KFM dl_toolbox=not_installed\n');
end
fprintf('KFM total_s=%.1f\n', toc(t0));
end
function s = pf(ok)
if ok, s = 'PASS'; else, s = 'FAIL'; end
end
