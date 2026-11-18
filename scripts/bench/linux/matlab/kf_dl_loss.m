function [loss, grads] = kf_dl_loss(dnet, X, Y)
% Mean squared error of a dlnetwork and its gradients (the function people hand to dlfeval and dlaccelerate).
P = forward(dnet, X);
loss = mean((P - Y) .^ 2, 'all');
grads = dlgradient(loss, dnet.Learnables);
end
