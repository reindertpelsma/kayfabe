function P = kf_dl_predict(net, X, mb, usegpu)
% predict() of a dlnetwork over X (h-by-w-by-c-by-N) in mini-batches of mb; with usegpu the batches and the learnables are on the GPU and the result stays a gpuArray.
if usegpu
    net = dlupdate(@gpuArray, net);
end
n = size(X, 4);
outs = {};
for s = 1:mb:n
    Xb = X(:, :, :, s:min(s + mb - 1, n));
    if usegpu
        Xb = gpuArray(Xb);
    end
    outs{end+1} = extractdata(predict(net, dlarray(Xb, 'SSCB'))); %#ok<AGROW>
end
P = cat(2, outs{:});
end
