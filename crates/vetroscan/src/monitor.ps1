# SPDX-License-Identifier: GPL-2.0-or-later
# Asks Windows for the monitor of a window and its colour profile (screen.rs adds the
# call with the window's process). Compiled by PowerShell: no `unsafe` in VetroScan.
$code = @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class VetroScanIcm {
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    public struct MonitorInfo {
        public int Size;
        public int L, T, R, B, WL, WT, WR, WB;
        public uint Flags;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string Device;
    }
    [DllImport("user32.dll")] static extern IntPtr MonitorFromWindow(IntPtr window, uint flags);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern bool GetMonitorInfo(IntPtr monitor, ref MonitorInfo info);
    [DllImport("gdi32.dll", CharSet = CharSet.Unicode)] static extern IntPtr CreateDC(string driver, string device, string output, IntPtr mode);
    [DllImport("gdi32.dll")] static extern bool DeleteDC(IntPtr dc);
    [DllImport("gdi32.dll", CharSet = CharSet.Unicode)] static extern bool GetICMProfile(IntPtr dc, ref uint size, StringBuilder name);
    public static string Of(IntPtr window) {
        IntPtr monitor = MonitorFromWindow(window, 2);
        MonitorInfo info = new MonitorInfo();
        info.Size = Marshal.SizeOf(info);
        if (!GetMonitorInfo(monitor, ref info)) return "";
        string profile = "";
        IntPtr dc = CreateDC("DISPLAY", info.Device, null, IntPtr.Zero);
        if (dc != IntPtr.Zero) {
            uint size = 1024;
            StringBuilder name = new StringBuilder(1024);
            if (GetICMProfile(dc, ref size, name)) profile = name.ToString();
            DeleteDC(dc);
        }
        return info.Device + "|" + profile;
    }
}
'@
Add-Type -TypeDefinition $code
