function c = kf_range(name, value, lo, hi)
% A statistic that must lie in a closed interval. The host judges it against ITS OWN interval for this name; lo and hi are recorded for the reader, never trusted.
c = struct();
c.name = name;
c.kind = 'range';
c.value = double(gather(value));
c.min = lo;
c.expected = hi;
c.guest_pass = isfinite(c.value) && c.value >= lo && c.value <= hi;
end
