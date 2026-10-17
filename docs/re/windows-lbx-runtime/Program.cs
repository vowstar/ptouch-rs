using System.Runtime.Loader;
const string editor = @"C:\Program Files (x86)\Brother\P-touch Editor\6";
Environment.CurrentDirectory = editor;
Environment.SetEnvironmentVariable("PATH", editor + ";" + Environment.GetEnvironmentVariable("PATH"));
AssemblyLoadContext.Default.Resolving += (_, name) =>
{
    var candidate = Path.Combine(editor, name.Name + ".dll");
    return File.Exists(candidate) ? AssemblyLoadContext.Default.LoadFromAssemblyPath(candidate) : null;
};

try
{
    var appAssembly = AssemblyLoadContext.Default.LoadFromAssemblyPath(Path.Combine(editor, "PtouchEditor6.Wpf.dll"));
    var infoType = appAssembly.GetType("PtouchEditor6.Wpf.Services.AssemblyInfoService", throwOnError: true)!;
    var info = Activator.CreateInstance(infoType)!;
    var appName = (string)infoType.GetProperty("AppNameVersionOS")!.GetValue(info)!;
    var lbxAssembly = AssemblyLoadContext.Default.LoadFromAssemblyPath(Path.Combine(editor, "LibLbx.DotNet.dll"));
    var lib = lbxAssembly.GetType("LibLbx.LibLbx", throwOnError: true)!;
    var initialized = lib.GetMethod("Initialize", new[] { typeof(int), typeof(string), typeof(string) })!
        .Invoke(null, new object[] { 1033, appName, Path.Combine(editor, "Fonts") });
    Console.WriteLine($"initialize={initialized}; app={appName}");
    var factory = lbxAssembly.GetType("LibLbx.LbxFactory", throwOnError: true)!;
    var load = factory.GetMethod("CreateFromPath", new[] { typeof(string) })!;
    foreach (var path in args)
    {
        try
        {
            var lbx = load.Invoke(null, new object[] { path })!;
            Console.WriteLine($"file={Path.GetFileName(path)}");
            foreach (var name in new[] { "LbxVersion", "Generator", "CurrentModelName", "CurrentPrinterName", "CurrentPrinterId", "StoredPrinterID", "MediaId", "CurrentLayoutSheetName", "TextureCount", "PatternCount" })
            {
                var property = lbx.GetType().GetProperty(name);
                if (property is null) continue;
                try { Console.WriteLine($"  {name}={property.GetValue(lbx)}"); }
                catch (Exception ex) { Console.WriteLine($"  {name}=ERROR:{ex.GetBaseException().GetType().Name}:{ex.GetBaseException().Message}"); }
            }
            var getObjects = lbx.GetType().GetMethod("GetPtObjects", Type.EmptyTypes);
            if (getObjects?.Invoke(lbx, null) is Array objects)
            {
                Console.WriteLine($"  ObjectCount={objects.Length}");
                for (var index = 0; index < objects.Length; index++)
                {
                    var item = objects.GetValue(index);
                    if (item is null) continue;
                    Console.WriteLine($"  Object[{index}] type={item.GetType().FullName}");
                    foreach (var name in new[] { "ClassName", "Text", "Position", "Angle", "ObjectName" })
                    {
                        var property = item.GetType().GetProperty(name);
                        if (property is null || !property.CanRead) continue;
                        try { Console.WriteLine($"    {name}={property.GetValue(item)}"); }
                        catch (Exception ex) { Console.WriteLine($"    {name}=ERROR:{ex.GetBaseException().GetType().Name}:{ex.GetBaseException().Message}"); }
                    }
                    var getBarcodeProperty = item.GetType().GetMethod("GetBarcodeProperty", Type.EmptyTypes);
                    if (getBarcodeProperty?.Invoke(item, null) is not { } barcode) continue;
                    try
                    {
                        var modeOverride = Environment.GetEnvironmentVariable("PT_QR_MODE_OVERRIDE");
                        var dataOverride = Environment.GetEnvironmentVariable("PT_QR_DATA_OVERRIDE");
                        var protocol = barcode.GetType().GetProperty("Protocol")?.GetValue(barcode)?.ToString();
                        var qrData = string.Equals(protocol, "QRCODE", StringComparison.OrdinalIgnoreCase)
                            ? barcode.GetType().GetProperty("QrcodeData")?.GetValue(barcode)
                            : null;
                        var modeProperty = qrData?.GetType().GetProperty("Mode");
                        var changed = false;
                        if (!string.IsNullOrWhiteSpace(modeOverride) && modeProperty?.CanWrite == true)
                        {
                            var mode = Enum.Parse(modeProperty.PropertyType, modeOverride, ignoreCase: true);
                            modeProperty.SetValue(qrData, mode);
                            Console.WriteLine($"    RuntimeModeOverride={mode}");
                            changed = true;
                        }
                        if (qrData is not null && !string.IsNullOrWhiteSpace(dataOverride))
                        {
                            var dataProperty = barcode.GetType().GetProperty("Data");
                            if (dataProperty?.CanWrite == true)
                            {
                                dataProperty.SetValue(barcode, dataOverride);
                                Console.WriteLine($"    RuntimeDataOverride={dataOverride}");
                                changed = true;
                            }
                        }
                        if (changed)
                            item.GetType().GetMethod("SetBarcodeProperty")?.Invoke(item, new[] { barcode });
                        foreach (var name in new[] { "Protocol", "Data" })
                        {
                            var property = barcode.GetType().GetProperty(name);
                            if (property?.CanRead == true)
                                Console.WriteLine($"    Barcode.{name}={property.GetValue(barcode)}");
                        }
                        foreach (var groupName in new[] { "BarcodeData", "QrcodeData" })
                        {
                            var group = barcode.GetType().GetProperty(groupName)?.GetValue(barcode);
                            if (group is null) continue;
                            foreach (var property in group.GetType().GetProperties())
                            {
                                if (!property.CanRead || property.GetIndexParameters().Length != 0) continue;
                                var value = property.GetValue(group);
                                if (value is null || value.GetType().IsPrimitive || value.GetType().IsEnum || value is string)
                                    Console.WriteLine($"    {groupName}.{property.Name}={value}");
                            }
                        }
                    }
                    finally
                    {
                        (barcode as IDisposable)?.Dispose();
                    }
                }
            }
            var getPrintingImage = lbx.GetType().GetMethod("GetPrintingImage", Type.EmptyTypes);
            if (getPrintingImage?.Invoke(lbx, null) is MemoryStream rendered)
            {
                using (rendered)
                {
                    var bytes = rendered.ToArray();
                    var output = Path.Combine(
                        Environment.GetFolderPath(Environment.SpecialFolder.UserProfile),
                        "ptouch-object-probe",
                        $"rendered-{Environment.GetEnvironmentVariable("PT_QR_DATA_OVERRIDE") ?? Environment.GetEnvironmentVariable("PT_QR_MODE_OVERRIDE") ?? "original"}-{Path.GetFileNameWithoutExtension(path)}.bin");
                    File.WriteAllBytes(output, bytes);
                    Console.WriteLine($"  RenderedImage={output}; bytes={bytes.Length}; signature={Convert.ToHexString(bytes.Take(8).ToArray())}");
                }
            }
            (lbx as IDisposable)?.Dispose();
        }
        catch (Exception ex) { Console.WriteLine($"file={Path.GetFileName(path)} ERROR:{ex.GetBaseException().GetType().Name}:{ex.GetBaseException().Message}"); }
    }
    lib.GetMethod("Uninitialize", Type.EmptyTypes)?.Invoke(null, null);
}
catch (Exception ex)
{
    Console.Error.WriteLine(ex.ToString());
    Environment.ExitCode = 1;
}
