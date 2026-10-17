# Read-only probe: send ESC i S (status request) to the first Brother USBPRINT interface and dump the 32-byte reply.
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;
public static class UsbPrint {
    static readonly Guid GUID_USBPRINT = new Guid("28d78fad-5a12-11d1-ae5b-0000f803a8c2");
    [StructLayout(LayoutKind.Sequential)] struct SP_DEVICE_INTERFACE_DATA { public int cbSize; public Guid g; public int flags; public IntPtr reserved; }
    [DllImport("setupapi.dll", SetLastError=true)] static extern IntPtr SetupDiGetClassDevsW(ref Guid g, IntPtr e, IntPtr h, int flags);
    [DllImport("setupapi.dll", SetLastError=true)] static extern bool SetupDiEnumDeviceInterfaces(IntPtr set, IntPtr di, ref Guid g, int idx, ref SP_DEVICE_INTERFACE_DATA d);
    [DllImport("setupapi.dll", SetLastError=true, CharSet=CharSet.Unicode)] static extern bool SetupDiGetDeviceInterfaceDetailW(IntPtr set, ref SP_DEVICE_INTERFACE_DATA d, IntPtr detail, int size, out int req, IntPtr di);
    [DllImport("setupapi.dll")] static extern bool SetupDiDestroyDeviceInfoList(IntPtr set);
    [DllImport("kernel32.dll", SetLastError=true, CharSet=CharSet.Unicode)] public static extern SafeFileHandle CreateFileW(string p, uint access, uint share, IntPtr sa, uint disp, uint flags, IntPtr t);
    [DllImport("kernel32.dll", SetLastError=true)] public static extern bool WriteFile(SafeFileHandle h, byte[] b, int n, out int w, IntPtr o);
    [DllImport("kernel32.dll", SetLastError=true)] public static extern bool ReadFile(SafeFileHandle h, byte[] b, int n, out int r, IntPtr o);
    public static string[] Paths() {
        var g = GUID_USBPRINT; var list = new System.Collections.Generic.List<string>();
        IntPtr set = SetupDiGetClassDevsW(ref g, IntPtr.Zero, IntPtr.Zero, 0x12); // DIGCF_PRESENT|DIGCF_DEVICEINTERFACE
        for (int i = 0; ; i++) {
            var d = new SP_DEVICE_INTERFACE_DATA(); d.cbSize = Marshal.SizeOf(d);
            if (!SetupDiEnumDeviceInterfaces(set, IntPtr.Zero, ref g, i, ref d)) break;
            int req; SetupDiGetDeviceInterfaceDetailW(set, ref d, IntPtr.Zero, 0, out req, IntPtr.Zero);
            IntPtr buf = Marshal.AllocHGlobal(req);
            Marshal.WriteInt32(buf, IntPtr.Size == 8 ? 8 : 6);
            if (SetupDiGetDeviceInterfaceDetailW(set, ref d, buf, req, out req, IntPtr.Zero)) list.Add(Marshal.PtrToStringUni(buf + 4));
            Marshal.FreeHGlobal(buf);
        }
        SetupDiDestroyDeviceInfoList(set); return list.ToArray();
    }
}
'@
$paths = [UsbPrint]::Paths()
"Interfaces:"; $paths | ForEach-Object { "  $_" }
$p = $paths | Where-Object { $_ -match 'vid_04f9' } | Select-Object -First 1
if (-not $p) { throw "No Brother USBPRINT interface present" }
$h = [UsbPrint]::CreateFileW($p, [uint32]3221225472, 3, [IntPtr]::Zero, 3, 0, [IntPtr]::Zero)  # GENERIC_READ|WRITE, OPEN_EXISTING
if ($null -eq $h -or $h.IsInvalid) { Write-Error "CreateFile failed: $([Runtime.InteropServices.Marshal]::GetLastWin32Error())"; return }
$cmd = [byte[]](@(0) * 100 + @(0x1B, 0x40) + @(0x1B, 0x69, 0x53))   # invalidate, ESC @ initialize, ESC i S status request
$w = 0; [void][UsbPrint]::WriteFile($h, $cmd, $cmd.Length, [ref]$w, [IntPtr]::Zero); "Wrote $w bytes"
$buf = New-Object byte[] 64; $r = 0
for ($i = 0; $i -lt 20 -and $r -eq 0; $i++) { [void][UsbPrint]::ReadFile($h, $buf, 64, [ref]$r, [IntPtr]::Zero); if ($r -eq 0) { Start-Sleep -Milliseconds 100 } }
$h.Dispose()
"Read $r bytes:"; ($buf[0..([Math]::Max($r,1)-1)] | ForEach-Object { '{0:X2}' -f $_ }) -join ' '
