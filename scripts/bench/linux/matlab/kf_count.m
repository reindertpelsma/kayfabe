function c = kf_count(name, value, expected)
% A count that must equal an expected number exactly (for example mismatched elements: expected 0).
c = struct();
c.name = name;
c.kind = 'count';
c.value = double(gather(value));
c.expected = expected;
c.guest_pass = c.value == expected;
end
