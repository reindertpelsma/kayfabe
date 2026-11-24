function c = kf_atleast(name, value, minimum)
c = struct();
c.name = name;
c.kind = 'atleast';
c.value = double(gather(value));
c.min = minimum;
c.guest_pass = isfinite(c.value) && c.value >= minimum;
end
