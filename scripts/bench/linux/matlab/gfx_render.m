% gfx_render: the graphics stack of the guest as MATLAB uses it: figures that are never shown, an exact-area test (a red disc filling the unit square must cover pi/4 of the
% exported image), a lit surface computed on the GPU and exported (coverage and colour count), an isosurface of a GPU volume whose area is the sphere's. The renderer MATLAB
% reports is RECORDED (graphics renderer, vendor, device, hardware support level, whether a display is present); only when a display is present, so that hardware
% rendering is possible at all, is a software renderer a failure (check gfx_renderer_software). Without a display the row records what happens and judges the pictures.
% Falsifier: the disc area or the isosurface area misses the host's interval, the surface export is blank, or (display present) the renderer is software.
row = 'gfx_render';
try
    d = gpuDevice(1);
    checks = {};
    doc = struct();
    wd = fullfile(kf_outdir(), 'gfxwork');
    if ~isfolder(wd)
        mkdir(wd);
    end
    rinfo = struct('graphics_renderer', '', 'vendor', '', 'renderer_device', '', 'hardware_support_level', '', 'display_present', ~isempty(getenv('DISPLAY')), ...
        'hardware_assertion_possible', ~isempty(getenv('DISPLAY')), 'software_renderer', false);
    try
        [X, Y] = meshgrid(linspace(-3, 3, 512));
        f = @(X, Y) 3 * (1 - X) .^ 2 .* exp(-X .^ 2 - (Y + 1) .^ 2) - 10 * (X / 5 - X .^ 3 - Y .^ 5) .* exp(-X .^ 2 - Y .^ 2) - exp(-(X + 1) .^ 2 - Y .^ 2) / 3;
        Zg = f(gpuArray(X), gpuArray(Y));
        checks{end+1} = kf_cmp('surface_double_512', Zg, f(X, Y), kf_tol('double'));
        checks{end+1} = kf_count('gfx_compute_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('gfx_compute_status', ME);
    end
    try
        fig = figure('Visible', 'off', 'Color', 'w', 'Position', [100 100 500 500]);
        ax = axes(fig, 'Position', [0 0 1 1]);
        t = linspace(0, 2 * pi, 2000);
        fill(ax, cos(t), sin(t), 'r', 'EdgeColor', 'none');
        axis(ax, 'off');
        xlim(ax, [-1 1]);
        ylim(ax, [-1 1]);
        try
            ri = rendererinfo(ax);
            rinfo.graphics_renderer = char(string(ri.GraphicsRenderer));
            rinfo.vendor = char(string(ri.Vendor));
            rinfo.renderer_device = char(string(ri.RendererDevice));
            rinfo.hardware_support_level = char(string(ri.Details.HardwareSupportLevel));
            all3 = lower([rinfo.graphics_renderer ' ' rinfo.renderer_device]);
            rinfo.software_renderer = contains(all3, 'swiftshader') || contains(all3, 'llvmpipe') || contains(all3, 'software');
        catch
        end
        f1 = fullfile(wd, 'disc.png');
        exportgraphics(fig, f1, 'Resolution', 96);
        im = imread(f1);
        red = im(:, :, 1) > 200 & im(:, :, 2) < 60 & im(:, :, 3) < 60;
        checks{end+1} = kf_range('gfx_circle_area_fraction', mean(red(:)), 0.77, 0.80);
        close(fig);
        fig = figure('Visible', 'off', 'Color', 'w', 'Position', [100 100 640 480]);
        ax = axes(fig);
        surf(ax, X, Y, gather(Zg), 'EdgeColor', 'none');
        lighting(ax, 'gouraud');
        camlight(ax);
        colorbar(ax);
        f2 = fullfile(wd, 'surf.png');
        exportgraphics(fig, f2, 'Resolution', 96);
        im = imread(f2);
        checks{end+1} = kf_range('gfx_nonwhite_fraction', mean(any(im < 250, 3), 'all'), 0.15, 0.97);
        checks{end+1} = kf_atleast('gfx_distinct_colors', size(unique(reshape(im, [], size(im, 3)), 'rows'), 1), 64);
        close(fig);
        checks{end+1} = kf_count('gfx_figure_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('gfx_figure_status', ME);
    end
    try
        [X3, Y3, Z3] = meshgrid(linspace(-1, 1, 96));
        vg = gpuArray(X3) .^ 2 + gpuArray(Y3) .^ 2 + gpuArray(Z3) .^ 2;
        checks{end+1} = kf_gpu('gfx_gpu_proof', vg);
        fv = isosurface(X3, Y3, Z3, gather(vg), 0.25);
        V = fv.vertices;
        F = fv.faces;
        a = V(F(:, 2), :) - V(F(:, 1), :);
        b = V(F(:, 3), :) - V(F(:, 1), :);
        area = 0.5 * sum(vecnorm(cross(a, b, 2), 2, 2));
        checks{end+1} = kf_atleast('isosurface_vertices', size(V, 1), 1000);
        checks{end+1} = kf_range('gfx_isosurface_area_ratio', area / (4 * pi * 0.25), 0.98, 1.02);
        checks{end+1} = kf_count('gfx_iso_status', 0, 0);
    catch ME
        checks{end+1} = kf_err('gfx_iso_status', ME);
    end
    if rinfo.hardware_assertion_possible
        checks{end+1} = kf_count('gfx_renderer_software', double(rinfo.software_renderer), 0);
    end
    doc.status = 'done';
    doc.checks = checks;
    doc.renderer = rinfo;
    kf_write(row, doc);
catch ME
    kf_fail(row, ME);
    rethrow(ME);
end
