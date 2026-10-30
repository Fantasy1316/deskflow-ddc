param([ValidateSet('list','get','set','host','peer','pointer','screens','serve')][string]$Action = 'list', [string]$MonitorId = '', [int]$Value = 0, [int]$X = 0, [int]$Y = 0)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;
public static class ScreenBridgeNative {
  [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)] public struct Physical { public IntPtr handle; [MarshalAs(UnmanagedType.ByValTStr, SizeConst=128)] public string description; }
  [StructLayout(LayoutKind.Sequential)] public struct Rect { public int left, top, right, bottom; }
  [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)] public struct MonitorInfo { public int size; public Rect monitor, work; public uint flags; [MarshalAs(UnmanagedType.ByValTStr, SizeConst=32)] public string device; }
  [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)] public struct DisplayDevice {
    public int size;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst=32)] public string name;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst=128)] public string description;
    public uint flags;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst=128)] public string id;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst=128)] public string key;
  }
  public class Item { public string id; public string name; public uint? currentInput; public bool readable; }
  private delegate bool Callback(IntPtr monitor, IntPtr hdc, IntPtr rect, IntPtr data);
  [DllImport("user32.dll")] private static extern bool EnumDisplayMonitors(IntPtr hdc, IntPtr clip, Callback callback, IntPtr data);
  [DllImport("user32.dll", CharSet=CharSet.Unicode, EntryPoint="GetMonitorInfoW")] private static extern bool GetMonitorInfo(IntPtr monitor, ref MonitorInfo info);
  [DllImport("user32.dll", CharSet=CharSet.Unicode, EntryPoint="EnumDisplayDevicesW")] private static extern bool EnumDisplayDevices(string device, uint index, ref DisplayDevice info, uint flags);
  [DllImport("dxva2.dll", SetLastError=true)] private static extern bool GetNumberOfPhysicalMonitorsFromHMONITOR(IntPtr monitor, out uint count);
  [DllImport("dxva2.dll", SetLastError=true)] private static extern bool GetPhysicalMonitorsFromHMONITOR(IntPtr monitor, uint count, [Out] Physical[] result);
  [DllImport("dxva2.dll", SetLastError=true)] private static extern bool DestroyPhysicalMonitors(uint count, Physical[] monitors);
  [DllImport("dxva2.dll", SetLastError=true)] private static extern bool GetVCPFeatureAndVCPFeatureReply(IntPtr monitor, byte code, out uint type, out uint current, out uint max);
  [DllImport("dxva2.dll", SetLastError=true)] private static extern bool SetVCPFeature(IntPtr monitor, byte code, uint value);
  [StructLayout(LayoutKind.Sequential)] private struct Input { public uint type; public InputUnion union; }
  [StructLayout(LayoutKind.Explicit)] private struct InputUnion { [FieldOffset(0)] public Keyboard keyboard; [FieldOffset(0)] public Mouse mouse; }
  [StructLayout(LayoutKind.Sequential)] private struct Keyboard { public ushort vk, scan; public uint flags, time; public UIntPtr extra; }
  [StructLayout(LayoutKind.Sequential)] private struct Mouse { public int x,y; public uint data,flags,time; public UIntPtr extra; }
  [DllImport("user32.dll", SetLastError=true)] private static extern uint SendInput(uint count, Input[] inputs, int size);
  [DllImport("user32.dll", SetLastError=true)] private static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] private static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
  public class Panel { public string id; public string name; public bool @internal = false; public Area bounds; }
  public class Area { public int x,y,width,height; }
  public static string StableScreenId(IEnumerable<string> interfaces) {
    // EDD_GET_DEVICE_INTERFACE_NAME returns the monitor device interface path,
    // rather than a session-specific HMONITOR or a reorderable DISPLAY number.
    // A cloned desktop can have several physical targets: include the complete,
    // sorted set so enumeration order cannot change its logical screen ID.
    var paths = new SortedSet<string>(StringComparer.Ordinal);
    foreach(var path in interfaces) {
      if(!String.IsNullOrWhiteSpace(path)) paths.Add(path.Trim().ToUpperInvariant());
    }
    if(paths.Count == 0) throw new Exception("Cannot identify display device; retry after the display is available");
    using(var hash = SHA256.Create()) {
      return "win-monitor-" + BitConverter.ToString(hash.ComputeHash(Encoding.UTF8.GetBytes(String.Join("\n", paths)))).Replace("-", "").ToLowerInvariant();
    }
  }
  public static Panel ReadPanel(IntPtr monitor, Func<IntPtr, MonitorInfo> readInfo, Func<string, IEnumerable<string>> readInterfaces) {
    // Resolve the live handle on every scan; only the device ID is persisted.
    var info = readInfo(monitor);
    return new Panel {id=StableScreenId(readInterfaces(info.device)),name=info.device,bounds=new Area {x=info.monitor.left,y=info.monitor.top,width=info.monitor.right-info.monitor.left,height=info.monitor.bottom-info.monitor.top}};
  }
  private static MonitorInfo ReadMonitorInfo(IntPtr monitor) {
    var info = new MonitorInfo {size=Marshal.SizeOf(typeof(MonitorInfo))};
    if(!GetMonitorInfo(monitor,ref info)) throw new Win32Exception(Marshal.GetLastWin32Error());
    return info;
  }
  private static IEnumerable<string> MonitorInterfaces(string device) {
    var paths = new List<string>();
    for(uint index=0;;index++) {
      var info = new DisplayDevice {size=Marshal.SizeOf(typeof(DisplayDevice))};
      if(!EnumDisplayDevices(device,index,ref info,1)) break; // EDD_GET_DEVICE_INTERFACE_NAME
      if((info.flags & 1) != 0 && !String.IsNullOrWhiteSpace(info.id)) paths.Add(info.id); // DISPLAY_DEVICE_ACTIVE
    }
    return paths;
  }
  public static List<Panel> Screens() {
    var previous = SetThreadDpiAwarenessContext(new IntPtr(-4));
    try {
      var result = new List<Panel>();
      Exception failure = null;
      Callback callback = delegate(IntPtr monitor,IntPtr hdc,IntPtr rect,IntPtr data) {
        // Do not let managed exceptions cross the native callback boundary.
        try { result.Add(ReadPanel(monitor,ReadMonitorInfo,MonitorInterfaces)); }
        catch(Exception ex) { failure=ex; }
        return true;
      };
      if(!EnumDisplayMonitors(IntPtr.Zero,IntPtr.Zero,callback,IntPtr.Zero)) throw new Win32Exception(Marshal.GetLastWin32Error());
      if(failure != null) throw failure;
      return result;
    } finally { if(previous != IntPtr.Zero) SetThreadDpiAwarenessContext(previous); }
  }
  public static void Position(int x, int y) {
    var previous = SetThreadDpiAwarenessContext(new IntPtr(-4));
    try {
      if(!SetCursorPos(x,y)) throw new Win32Exception(Marshal.GetLastWin32Error());
      // Deliver motion through the low-level hook as well as positioning the OS
      // cursor, so Deskflow observes a provider-side native edge crossing.
      var move = new Input { type=0, union=new InputUnion { mouse=new Mouse { flags=1 } } };
      if(SendInput(1,new Input[] {move},Marshal.SizeOf(typeof(Input))) != 1) throw new Win32Exception(Marshal.GetLastWin32Error());
    }
    finally { if(previous != IntPtr.Zero) SetThreadDpiAwarenessContext(previous); }
  }
  public static void Hotkey(bool host) {
    ushort[] keys = {0x11, 0x12, 0x10, (ushort)(host ? 0x81 : 0x82)}; // Ctrl Alt Shift F18/F19
    var inputs = new List<Input>();
    foreach(var key in keys) inputs.Add(new Input { type=1, union=new InputUnion { keyboard=new Keyboard { vk=key } } });
    for(int i=keys.Length-1;i>=0;i--) inputs.Add(new Input { type=1, union=new InputUnion { keyboard=new Keyboard { vk=keys[i], flags=2 } } });
    if(SendInput((uint)inputs.Count, inputs.ToArray(), Marshal.SizeOf(typeof(Input))) != inputs.Count) throw new Win32Exception(Marshal.GetLastWin32Error());
  }
  public static List<Item> Monitors(string selected, int? value) {
    var result = new List<Item>(); Exception failure=null; bool matched=false;
    Callback callback = delegate(IntPtr monitor, IntPtr hdc, IntPtr rect, IntPtr data) {
      uint count; if(!GetNumberOfPhysicalMonitorsFromHMONITOR(monitor,out count)) return true;
      var physical=new Physical[count]; if(!GetPhysicalMonitorsFromHMONITOR(monitor,count,physical)) return true;
      try {
        var info=new MonitorInfo {size=Marshal.SizeOf(typeof(MonitorInfo))};
        if(!GetMonitorInfo(monitor,ref info)) return true;
        for(int i=0;i<count;i++) {
          string id=info.device+"#"+i;
          if(value.HasValue) {
            if(id==selected) { matched=true; if(!SetVCPFeature(physical[i].handle,0x60,(uint)value.Value)) failure=new Win32Exception(Marshal.GetLastWin32Error(), "DDC input write failed"); }
          } else if(String.IsNullOrEmpty(selected) || id==selected) {
            uint type,current,max; bool readable=GetVCPFeatureAndVCPFeatureReply(physical[i].handle,0x60,out type,out current,out max);
            result.Add(new Item { id=id,name=physical[i].description,currentInput=readable?(uint?)current:null,readable=readable });
          }
        }
      } catch(Exception ex) {failure=ex;} finally {DestroyPhysicalMonitors(count,physical);}
      return true;
    };
    if(!EnumDisplayMonitors(IntPtr.Zero,IntPtr.Zero,callback,IntPtr.Zero)) throw new Win32Exception(Marshal.GetLastWin32Error());
    if(failure!=null) throw failure;
    if(value.HasValue && !matched) throw new Exception("Selected monitor is not connected");
    return result;
  }
}
'@
function Invoke-NativeCommand([string]$Command, [string]$Monitor, [int]$InputValue, [int]$PointerX, [int]$PointerY) {
  switch ($Command) {
    'screens' { $items = [ScreenBridgeNative]::Screens(); return ,@($items.ToArray()) }
    'list' { $items = [ScreenBridgeNative]::Monitors('', $null); return ,@($items.ToArray()) }
    'get' { $items = [ScreenBridgeNative]::Monitors($Monitor, $null); if ($items.Count -eq 1) { return $items[0].currentInput }; return $null }
    'set' { if ($InputValue -lt 0 -or $InputValue -gt 65535) { throw 'Invalid input value' }; [void][ScreenBridgeNative]::Monitors($Monitor, $InputValue); return @{ sent = $true } }
    'host' { [ScreenBridgeNative]::Hotkey($true); return @{ sent = $true } }
    'pointer' { [ScreenBridgeNative]::Position($PointerX, $PointerY); return @{ positioned = $true } }
    'peer' { [ScreenBridgeNative]::Hotkey($false); return @{ sent = $true } }
    default { throw 'Unsupported native command' }
  }
}
try {
  if ($Action -eq 'serve') {
    # Compile the native bindings once. Only read private stdin; EOF exits when
    # the owning app dies. Never evaluate request strings as PowerShell code.
    while ($null -ne ($line = [Console]::ReadLine())) {
      $request = $null
      try {
        if ($line.Length -gt 16384) { throw 'Native request too large' }
        $request = ConvertFrom-Json -InputObject $line
        if ($null -eq $request.id -or $request.action -notin @('list','get','set','host','peer','pointer','screens')) { throw 'Invalid native request' }
        $result = Invoke-NativeCommand $request.action $request.monitorId $request.value $request.x $request.y
        $response = @{ id = $request.id; ok = $true; result = $result }
      } catch { $response = @{ id = $request.id; ok = $false; error = $_.Exception.Message } }
      [Console]::Out.WriteLine((ConvertTo-Json -InputObject $response -Depth 6 -Compress))
      [Console]::Out.Flush()
    }
  } else {
    $result = Invoke-NativeCommand $Action $MonitorId $Value $X $Y
    ConvertTo-Json -InputObject $result -Depth 6 -Compress
  }
} catch { [Console]::Error.WriteLine($_.Exception.Message); exit 1 }
