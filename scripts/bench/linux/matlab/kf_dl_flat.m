function v = kf_dl_flat(c)
% All the arrays of a learnables table or a cell array of dlarrays or numeric arrays as one column vector (on the device of the data).
if istable(c)
    c = c.Value;
end
v = cell(numel(c), 1);
for i = 1:numel(c)
    x = c{i};
    if isa(x, 'dlarray')
        x = extractdata(x);
    end
    v{i} = reshape(x, [], 1);
end
v = vertcat(v{:});
end
