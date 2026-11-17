function c = kf_cmp(name, xg, xc, tol)
% GPU result xg versus CPU result xc: fingerprints of both, the 2-norm of their difference and of the reference. tol is stated, not trusted:
% the host recomputes the agreement from these raw numbers with its own tolerance.
xg = gather(xg);
d = double(xg(:)) - double(xc(:));
c = struct();
c.name = name;
c.kind = 'cmp';
c.dtype = class(xc);
c.numel = numel(xc);
c.tol = tol;
c.gpu = kf_fp(xg);
c.cpu = kf_fp(xc);
c.diff = norm(d);
c.ref = norm(double(xc(:)));
c.guest_pass = isfinite(c.diff) && isfinite(c.ref) && c.diff <= tol * max(c.ref, 1e-300);
end
