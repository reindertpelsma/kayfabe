// Run against the test project using AfterFX.com -r. Writes only fixed fields.
(function () {
    function backend(value) {
        if (value === GpuAccelType.CUDA) return "cuda";
        if (value === GpuAccelType.OPENCL) return "opencl";
        if (value === GpuAccelType.SOFTWARE) return "software";
        return "unknown";
    }
    var version = String(app.version).match(/^\d+(\.\d+){1,4}/);
    var lines = ["version\t" + (version ? version[0] : "unknown")];
    if (app.project) lines.push("renderer\t" + backend(app.project.gpuAccelType));
    var available = app.availableGPUAccelTypes;
    if (available && available.length) {
        for (var i = 0; i < available.length; i++) {
            var name = backend(available[i]);
            if (name !== "unknown") lines.push("feature\tmercury-available." + name +
                "\tenabled\tAvailable renderer: " + name);
        }
    }
    var item = app.project && app.project.activeItem;
    if (item instanceof CompItem) {
        var advanced = item.renderer === "ADBE Advanced 3d";
        lines.push("feature\tadvanced-3d\t" + (advanced ? "enabled" : "disabled") +
            "\tActive comp renderer " + (advanced ? "is Advanced 3D" : "is another renderer"));
    }
    var file = new File(Folder.temp.fsName + "/kf-ae-observations.tsv");
    file.encoding = "UTF-8";
    if (!file.open("w")) throw new Error("Cannot open census output");
    file.write(lines.join("\n") + "\n");
    file.close();
}());
