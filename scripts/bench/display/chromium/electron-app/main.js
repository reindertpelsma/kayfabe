// Minimal Electron app standing in for a Chromium-based launcher (Minecraft Launcher class): a BrowserWindow,
// the remote-debugging port is given on the command line, the load comes from cdp_load.py.
const { app, BrowserWindow } = require('electron');
app.whenReady().then(() => {
  const w = new BrowserWindow({ width: 1600, height: 900, x: 0, y: 0, webPreferences: { backgroundThrottling: false } });
  w.loadURL('about:blank');
});
app.on('window-all-closed', () => app.quit());
