function [y, z] = kf_af_iter(x, c)
% The body of an arrayfun kernel (runs element-wise on the GPU): a fixed-count loop of a contraction map, then a branch. Contraction means GPU and CPU
% rounding cannot diverge; the branch is continuous at its switch point (both arms are zero there).
y = x;
for k = 1:50
    y = 0.5 * cos(y) + c;
end
if y > c
    z = sqrt(y - c);
else
    z = -log1p(c - y);
end
end
