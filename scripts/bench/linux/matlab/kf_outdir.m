function d = kf_outdir()
% Directory the rows write their JSON result to (created on demand). Never an account-dependent path in the JSON itself.
d = getenv('KF_ROWS_OUT');
if isempty(d)
    d = fullfile(getenv('HOME'), 'kfrows', 'out');
end
if ~isfolder(d)
    mkdir(d);
end
end
