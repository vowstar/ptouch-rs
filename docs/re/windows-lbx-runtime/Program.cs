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
