function c = kf_resid(name, value, tol)
% A scalar residual that must be small; computed from gathered data.
c = struct();
c.name = name;
c.kind = 'resid';
c.value = double(gather(value));
c.tol = tol;
c.guest_pass = isfinite(c.value) && c.value <= tol;
end
