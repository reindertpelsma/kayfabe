function [y, gx, gP] = kf_dl_probe(op, x, P)
% One deep-learning functional operation and the gradients of a fixed weighted sum of its output with respect to the input and the parameter cell P.
% Called through dlfeval, once on the CPU and once on the GPU, with identical data.
switch op
    case {'conv1d', 'conv2d', 'conv2d_s', 'conv3d'}
        y = dlconv(x, P{1}, P{2}, 'Padding', 'same');
    case 'transpconv2d'
        y = dltranspconv(x, P{1}, P{2}, 'Stride', 2);
    case 'maxpool2d'
        y = maxpool(x, 2, 'Stride', 2);
    case 'avgpool2d'
        y = avgpool(x, 2, 'Stride', 2);
    case 'batchnorm'
        y = batchnorm(x, P{1}, P{2});
    case 'groupnorm'
        y = groupnorm(x, 2, P{1}, P{2});
    case 'layernorm'
        y = layernorm(x, P{1}, P{2});
    case 'attention'
        y = attention(x, x, x, 2);
    case 'lstm'
        y = lstm(x, P{4}, P{5}, P{1}, P{2}, P{3});
    case 'gru'
        y = gru(x, P{4}, P{1}, P{2}, P{3}, 'ResetGateMode', 'before-multiplication');
    case 'gelu'
        y = gelu(x);
    case 'softmax_ce'
        t = zeros(size(x), 'like', extractdata(x));
        t(sub2ind(size(t), mod(0:size(t, 2) - 1, size(t, 1)) + 1, 1:size(t, 2))) = 1;
        y = crossentropy(softmax(x), dlarray(t, 'CB'));
    case 'fullyconnect'
        y = fullyconnect(x, P{1}, P{2});
    otherwise
        error('kf:probe:op', 'unknown op');
end
r = cast(reshape(cos(1:numel(y)), size(y)), 'like', extractdata(y));
s = sum(y .* r, 'all');
if isempty(P)
    gx = dlgradient(s, x);
    gP = {};
else
    [gx, gP] = dlgradient(s, x, P);
end
end
