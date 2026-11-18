// Probe the legacy DOM. Keep only numeric version and allowlisted System Info lines.
(function () {
    var info = app.systemInformation;
    if (typeof info !== "string") return;
    var lines = info.split(/\r\n|\r|\n/);
    var allowed = /^[ \t]*(useOpenCL|IsOpenCLGPUCapable|Use Graphics Processor):[ \t]*(0|1|true|false|on|off)[ \t]*$/i;
    var version = String(app.version).match(/^\d+(\.\d+){1,4}/);
    var out = version ? ["Adobe Photoshop Version: " + version[0]] : [];
    for (var i = 0; i < lines.length; i++) {
        if (allowed.test(lines[i])) out.push(lines[i]);
    }
    var file = new File(Folder.temp.fsName + "/kf-ps-system-info.txt");
    file.encoding = "UTF-8";
    if (!file.open("w")) throw new Error("Cannot open census output");
    file.write(out.join("\n") + "\n");
    file.close();
}());
