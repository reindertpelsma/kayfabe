function v = kf_fp(x)
% A four-number fingerprint of an array: 2-norm, sum, first and last element (real parts), computed in double on the CPU.
x = double(gather(x));
x = x(:);
if isempty(x)
    v = [0 0 0 0];
    return
end
v = [norm(x), real(sum(x)), real(x(1)), real(x(end))];
end
