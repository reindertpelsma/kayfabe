function s = kf_long_kernel(n)
% A long run of matrix products on the pool worker's GPU (the worker is killed during this loop).
A = rand(2048, 2048, 'single', 'gpuArray');
s = zeros(1, 1, 'single', 'gpuArray');
for k = 1:n
    B = A * A;
    s = s + B(1, 1);
end
s = gather(s);
end
