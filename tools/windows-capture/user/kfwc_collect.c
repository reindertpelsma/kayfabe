/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 *
 * kfwc_collect.exe: user-mode companion of kfwc.sys.
 *
 *   kfwc_collect.exe OUTDIR [--seconds N] [--stop-file PATH] [--decode-tag TAG]
 *                    [--layout auto|pre610|610] [--no-mailbox] [--no-scan] [--gpu N]
 *                    [--status]
 *
 * Opens \\.\KayfabeWinCapture read-only, starts a fresh stream (record counter 0), appends every
 * record to OUTDIR\capture.kfgspcap after the 128-byte KFGSPCAP header, stops (the driver writes
 * END), drains, then checks the file it wrote and writes OUTDIR\cell.json.
 *
 * What is recorded about the machine: PCI vendor/device/revision of the die (from the driver),
 * the Windows build number, and the NVIDIA display driver's DriverVersion string. Nothing else:
 * no UUID, serial, VBIOS, MAC, PCI subsystem id, host name, user name or path. This file does
 * not read MatchingDeviceId or any hardware id for exactly that reason (tests/guard_readonly.sh
 * greps for the banned words).
 *
 * Exit codes: 0 stream complete (END seen, framing ok); 1 error; 2 usage; 3 END missing or file
 * check failed; 4 nothing captured (no ELEMENTS record).
 */
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <winioctl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "../lib/kfwc_meta.h"

static volatile LONG g_interrupted;
static BOOL WINAPI on_ctrl(DWORD ev)
{
    (void)ev;
    InterlockedExchange(&g_interrupted, 1);
    return TRUE;
}

static int ioctl_out(HANDLE h, DWORD code, void *out, DWORD out_len, DWORD *got)
{
    DWORD n = 0;
    BOOL ok = DeviceIoControl(h, code, NULL, 0, out, out_len, &n, NULL);
    if (got)
        *got = n;
    return ok ? 0 : 1;
}

/* ---- registry (version strings only) ------------------------------------------------- */
static int reg_sz(HKEY root, const char *path, const char *name, char *out, DWORD cap)
{
    HKEY k;
    DWORD type = 0, n = cap - 1;
    LONG r = RegOpenKeyExA(root, path, 0, KEY_READ | KEY_WOW64_64KEY, &k);
    if (r != ERROR_SUCCESS)
        return 1;
    r = RegQueryValueExA(k, name, NULL, &type, (LPBYTE)out, &n);
    RegCloseKey(k);
    if (r != ERROR_SUCCESS || (type != REG_SZ && type != REG_DWORD))
        return 1;
    if (type == REG_DWORD) {
        DWORD v = 0;
        memcpy(&v, out, sizeof(v) < n ? sizeof(v) : n);
        snprintf(out, cap, "%lu", (unsigned long)v);
        return 0;
    }
    out[n < cap ? n : cap - 1] = 0;
    return 0;
}

/* The NVIDIA display driver's DriverVersion: first display-class instance whose ProviderName is
 * NVIDIA. Reads exactly two values from it. */
static int nvidia_driver_version(char *out, DWORD cap)
{
    const char *base = "SYSTEM\\CurrentControlSet\\Control\\Class\\{4d36e968-e325-11ce-bfc1-08002be10318}";
    int i;
    for (i = 0; i < 64; i++) {
        char path[256], prov[64];
        snprintf(path, sizeof(path), "%s\\%04d", base, i);
        if (reg_sz(HKEY_LOCAL_MACHINE, path, "ProviderName", prov, sizeof(prov)))
            continue;
        if (_stricmp(prov, "NVIDIA") != 0)
            continue;
        return reg_sz(HKEY_LOCAL_MACHINE, path, "DriverVersion", out, cap);
    }
    return 1;
}

/* ---- file checks ---------------------------------------------------------------------- */
static void check_file(const char *path, kfwc_meta *m)
{
    FILE *f = fopen(path, "rb");
    unsigned char *buf;
    long size;
    kfwc_cap_file fh;
    kfwc_u32 pos = KFWC_CAP_FILE_HEADER_BYTES, expect = 0;
    int ok = 1;
    m->file_ok = 0;
    if (!f)
        return;
    fseek(f, 0, SEEK_END);
    size = ftell(f);
    fseek(f, 0, SEEK_SET);
    buf = (unsigned char *)malloc((size_t)size + 1);
    if (!buf || fread(buf, 1, (size_t)size, f) != (size_t)size) {
        fclose(f);
        free(buf);
        return;
    }
    fclose(f);
    if (kfwc_cap_file_parse(buf, (kfwc_u32)size, &fh) != KFWC_PARSE_OK)
        ok = 0;
    while (ok && pos < (kfwc_u32)size) {
        kfwc_cap_rec r;
        const kfwc_u8 *pl;
        kfwc_u32 used;
        if (kfwc_cap_rec_parse(buf + pos, (kfwc_u32)size - pos, &r, &pl, &used) != KFWC_PARSE_OK) {
            ok = 0;
            break;
        }
        if (r.counter != expect)
            ok = 0; /* a hole in the counter: a record was refused or lost; reported, not hidden */
        expect = r.counter + 1;
        m->records_total++;
        if (r.kind < 8)
            m->records_by_kind[r.kind]++;
        if (r.kind == KFWC_K_END) {
            m->end_seen = 1;
            if (r.entry_count != r.counter || pos + used != (kfwc_u32)size)
                ok = 0;
        }
        pos += used;
    }
    m->file_ok = ok && m->end_seen;
    free(buf);
}

static int stop_marker(const char *path)
{
    DWORD a = GetFileAttributesA(path);
    if (a != INVALID_FILE_ATTRIBUTES)
        return (a & FILE_ATTRIBUTE_DIRECTORY) ? -1 : 1;
    return (GetLastError() == ERROR_FILE_NOT_FOUND || GetLastError() == ERROR_PATH_NOT_FOUND) ? 0 : -1;
}

int main(int argc, char **argv)
{
    const char *outdir = NULL, *stop_file = NULL, *tag = "";
    unsigned long seconds = 600;
    kfwc_start_req req;
    HANDLE dev;
    kfwc_info info;
    DWORD n;
    char path[MAX_PATH], cell_path[MAX_PATH];
    FILE *out = NULL;
    unsigned char *buf;
    kfwc_cap_file fh;
    unsigned char hb[KFWC_CAP_FILE_HEADER_BYTES];
    int i, rc = 1, status_only = 0, caps_only = 0;
    ULONGLONG deadline;
    char build[64] = "", ubr[32] = "", dispver[32] = "", drvver[64] = "";
    static kfwc_meta meta;
    static kfwc_event events[KFWC_EVENT_SLOTS];

    memset(&req, 0, sizeof(req));
    req.abi = KFWC_DRIVER_ABI;
    req.use_mailbox = 1;
    req.use_scan = 1;
    req.poll_us = 1000;
    req.scan_period_us = 2000;
    req.scan_bytes_per_step = 1024u * 1024u;
    req.scan_delay_ms = 5000;
    req.max_msgs_per_step = 64;
    for (i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "--seconds") && i + 1 < argc)
            seconds = strtoul(argv[++i], NULL, 10);
        else if (!strcmp(argv[i], "--stop-file") && i + 1 < argc)
            stop_file = argv[++i];
        else if (!strcmp(argv[i], "--decode-tag") && i + 1 < argc)
            tag = argv[++i];
        else if (!strcmp(argv[i], "--layout") && i + 1 < argc) {
            const char *v = argv[++i];
            req.layout = !strcmp(v, "pre610") ? 1 : !strcmp(v, "610") ? 2 : 0;
        } else if (!strcmp(argv[i], "--no-mailbox"))
            req.use_mailbox = 0;
        else if (!strcmp(argv[i], "--no-scan"))
            req.use_scan = 0;
        else if (!strcmp(argv[i], "--no-backoff"))
            req.scan_backoff_initial_ms = KFWC_BACKOFF_OFF;
        else if (!strcmp(argv[i], "--backoff-ms") && i + 2 < argc) {
            req.scan_backoff_initial_ms = (kfwc_u32)strtoul(argv[++i], NULL, 10);
            req.scan_backoff_max_ms = (kfwc_u32)strtoul(argv[++i], NULL, 10);
        }
        else if (!strcmp(argv[i], "--gpu") && i + 1 < argc)
            req.gpu_index = (kfwc_u32)strtoul(argv[++i], NULL, 10);
        else if (!strcmp(argv[i], "--status"))
            status_only = 1;
        else if (!strcmp(argv[i], "--display-caps"))
            caps_only = 1;
        else if (argv[i][0] != '-' && !outdir)
            outdir = argv[i];
        else {
            fprintf(stderr, "unknown argument %s\n", argv[i]);
            return 2;
        }
    }
    if ((!outdir && !status_only && !caps_only) || (caps_only && (status_only || req.gpu_index != 0)) ||
        seconds == 0 || seconds > 86400) {
        fprintf(stderr, "usage: kfwc_collect OUTDIR [--seconds N] [--stop-file P] [--decode-tag T] [--layout auto|pre610|610]\n"
                        "                    [--no-mailbox] [--no-scan] [--no-backoff] [--backoff-ms INITIAL MAX] [--gpu N] | --status | --display-caps\n");
        return 2;
    }
    dev = CreateFileA(KFWC_DEVICE_USER_PATH, GENERIC_READ, 0, NULL, OPEN_EXISTING, 0, NULL);
    if (dev == INVALID_HANDLE_VALUE) {
        fprintf(stderr, "cannot open the device: Win32 %lu (elevated prompt? driver started?)\n", GetLastError());
        return 1;
    }
    if (ioctl_out(dev, KFWC_IOCTL_INFO, &info, sizeof(info), &n) || n != sizeof(info) ||
        info.driver_abi != KFWC_DRIVER_ABI || info.engine_abi != KFWC_ENGINE_ABI) {
        fprintf(stderr, "driver ABI mismatch or INFO failed\n");
        CloseHandle(dev);
        return 1;
    }
    if (caps_only) {
        kfwc_display_caps_out caps;
        unsigned int attempt, j, nonzero = 0;
        int ok = 0;
        /* BAR mapping requires 20 ms of stable PCI configuration. At most 21 attempts,
         * separated by 50 ms; every attempt reads at most the fixed 1024 words. */
        for (attempt = 0; attempt < 21u; attempt++) {
            if (!ioctl_out(dev, KFWC_IOCTL_DISPLAY_CAPS, &caps, sizeof(caps), &n) &&
                n == sizeof(caps) && caps.word_count == KFWC_DISPLAY_CAPS_WORDS) {
                ok = 1;
                break;
            }
            Sleep(50);
        }
        if (!ok) {
            fprintf(stderr, "display caps unavailable: Win32 %lu (new measurement driver, powered display engine?)\n", GetLastError());
            CloseHandle(dev);
            return 1;
        }
        nvidia_driver_version(drvver, sizeof(drvver));
        printf("# kf-display-caps-page v1\n# native Windows driver %s vendor 0x%04x device 0x%04x\n",
               drvver, (unsigned)caps.pci_vendor_id, (unsigned)caps.pci_device_id);
        printf("DECISIVE 0x00c 0x%08x\n", (unsigned)caps.words[3]);
        for (j = 0; j < 8u; j += 2u)
            printf("EVENCAPA %u 0x%03x 0x%08x\n", j, 0x780u + j * 0x20u,
                   (unsigned)caps.words[(0x780u + j * 0x20u) / 4u]);
        for (j = 0; j < KFWC_DISPLAY_CAPS_WORDS; j++) {
            if (caps.words[j]) {
                printf("WORD 0x%03x 0x%08x\n", j * 4u, (unsigned)caps.words[j]);
                nonzero++;
            }
        }
        printf("# end words=%u\n", nonzero);
        CloseHandle(dev);
        return 0;
    }
    if (status_only) {
        printf("{\"driver_abi\":%u,\"running\":%u}\n", (unsigned)info.driver_abi, (unsigned)info.running);
        CloseHandle(dev);
        return 0;
    }
    if (stop_file && stop_marker(stop_file) != 0) {
        fprintf(stderr, "stop marker must be absent at start\n");
        CloseHandle(dev);
        return 2;
    }
    snprintf(path, sizeof(path), "%s\\capture.kfgspcap", outdir);
    snprintf(cell_path, sizeof(cell_path), "%s\\cell.json", outdir);
    /* CREATE_NEW: never overwrite an earlier capture. */
    {
        HANDLE fh_handle = CreateFileA(path, GENERIC_WRITE, 0, NULL, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, NULL);
        if (fh_handle == INVALID_HANDLE_VALUE) {
            fprintf(stderr, "cannot create %s (exists?): Win32 %lu\n", path, GetLastError());
            CloseHandle(dev);
            return 1;
        }
        CloseHandle(fh_handle);
        out = fopen(path, "wb");
    }
    buf = (unsigned char *)malloc(KFWC_MAX_READ_BYTES);
    if (!out || !buf) {
        fprintf(stderr, "cannot open output or allocate\n");
        CloseHandle(dev);
        return 1;
    }
    SetConsoleCtrlHandler(on_ctrl, TRUE);

    if (!DeviceIoControl(dev, KFWC_IOCTL_START, &req, sizeof(req), NULL, 0, &n, NULL)) {
        fprintf(stderr, "START failed: Win32 %lu\n", GetLastError());
        goto done;
    }
    if (ioctl_out(dev, KFWC_IOCTL_INFO, &info, sizeof(info), &n) || n != sizeof(info))
        goto done;
    if (!info.gpu_count)
        fprintf(stderr, "warning: no NVIDIA display device found by the driver; die identity is empty\n");

    reg_sz(HKEY_LOCAL_MACHINE, "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion", "CurrentBuildNumber", build, sizeof(build));
    reg_sz(HKEY_LOCAL_MACHINE, "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion", "UBR", ubr, sizeof(ubr));
    reg_sz(HKEY_LOCAL_MACHINE, "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion", "DisplayVersion", dispver, sizeof(dispver));
    if (nvidia_driver_version(drvver, sizeof(drvver)))
        fprintf(stderr, "warning: no NVIDIA DriverVersion in the registry (driver not installed yet?)\n");

    memset(&fh, 0, sizeof(fh));
    fh.os_id = KFWC_CAP_OS_WINDOWS;
    fh.module_version = info.module_version;
    fh.pci_vendor_id = info.pci_vendor_id;
    fh.pci_device_id = info.pci_device_id;
    fh.pci_revision_id = info.pci_revision_id;
    fh.ts_hz = info.qpc_frequency;
    fh.ts_start = info.started_qpc;
    if (kfwc_cap_set_string(fh.driver_version, drvver))
        fprintf(stderr, "warning: driver version string had unprintable characters (dropped)\n");
    kfwc_cap_set_string(fh.decode_tag, tag);
    if (kfwc_cap_file_encode(&fh, hb) != KFWC_CAP_FILE_HEADER_BYTES || fwrite(hb, 1, sizeof(hb), out) != sizeof(hb)) {
        fprintf(stderr, "cannot write the file header\n");
        goto done;
    }

    deadline = GetTickCount64() + seconds * 1000ull;
    while (GetTickCount64() < deadline && !g_interrupted) {
        DWORD got = 0;
        if (stop_file && stop_marker(stop_file) != 0) {
            fprintf(stderr, "stop marker seen\n");
            break;
        }
        if (!ReadFile(dev, buf, KFWC_MAX_READ_BYTES, &got, NULL)) {
            fprintf(stderr, "ReadFile failed: Win32 %lu\n", GetLastError());
            goto done;
        }
        if (got && (fwrite(buf, 1, got, out) != got || fflush(out))) {
            fprintf(stderr, "write failed\n");
            goto done;
        }
        if (!got)
            Sleep(5);
    }
    if (!DeviceIoControl(dev, KFWC_IOCTL_STOP, NULL, 0, NULL, 0, &n, NULL)) {
        fprintf(stderr, "STOP failed: Win32 %lu\n", GetLastError());
        goto done;
    }
    for (;;) { /* drain what is left, END included */
        DWORD got = 0;
        if (!ReadFile(dev, buf, KFWC_MAX_READ_BYTES, &got, NULL))
            goto done;
        if (!got)
            break;
        if (fwrite(buf, 1, got, out) != got)
            goto done;
    }
    if (fclose(out)) {
        out = NULL;
        goto done;
    }
    out = NULL;

    memset(&meta, 0, sizeof(meta));
    meta.windows_build = build;
    meta.windows_ubr = ubr;
    meta.windows_display_version = dispver;
    meta.driver_version = fh.driver_version;
    meta.decode_tag = fh.decode_tag;
    ioctl_out(dev, KFWC_IOCTL_INFO, &meta.info, sizeof(meta.info), &n);
    ioctl_out(dev, KFWC_IOCTL_STATS, &meta.stats, sizeof(meta.stats), &n);
    if (!ioctl_out(dev, KFWC_IOCTL_EVENTS, events, sizeof(events), &n))
        meta.n_events = n / (DWORD)sizeof(kfwc_event);
    meta.events = events;
    check_file(path, &meta);
    {
        static char json[16384];
        size_t len = kfwc_meta_format(json, sizeof(json), &meta);
        HANDLE c = CreateFileA(cell_path, GENERIC_WRITE, 0, NULL, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, NULL);
        DWORD w = 0;
        if (!len || c == INVALID_HANDLE_VALUE || !WriteFile(c, json, (DWORD)len, &w, NULL) || w != len) {
            fprintf(stderr, "cannot write cell.json\n");
            if (c != INVALID_HANDLE_VALUE)
                CloseHandle(c);
            goto done;
        }
        CloseHandle(c);
    }
    printf("records %u (bind %u, elements %u, loss %u/%u/%u), end %s, file check %s\n", (unsigned)meta.records_total,
           (unsigned)meta.records_by_kind[1], (unsigned)meta.records_by_kind[2], (unsigned)meta.records_by_kind[3],
           (unsigned)meta.records_by_kind[4], (unsigned)meta.records_by_kind[5], meta.end_seen ? "yes" : "NO",
           meta.file_ok ? "ok" : "FAILED");
    rc = !meta.end_seen || !meta.file_ok ? 3 : meta.records_by_kind[2] == 0 ? 4 : 0;
done:
    if (out)
        fclose(out);
    free(buf);
    CloseHandle(dev);
    return rc;
}
