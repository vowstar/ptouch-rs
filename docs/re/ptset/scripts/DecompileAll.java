// Headless post-script: decompile every non-external function and write it to <outdir>/<program>.c
// Usage: -postScript DecompileAll.java <outdir>
import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileResults;
import ghidra.program.model.listing.Function;
import java.io.File;
import java.io.PrintWriter;

public class DecompileAll extends GhidraScript {
    @Override
    public void run() throws Exception {
        String outDir = getScriptArgs().length > 0 ? getScriptArgs()[0] : ".";
        File out = new File(outDir, currentProgram.getName() + ".c");
        DecompInterface ifc = new DecompInterface();
        ifc.openProgram(currentProgram);
        int ok = 0, fail = 0;
        try (PrintWriter w = new PrintWriter(out, "UTF-8")) {
            for (Function f : currentProgram.getFunctionManager().getFunctions(true)) {
                if (monitor.isCancelled()) break;
                if (f.isExternal() || f.isThunk()) continue;
                DecompileResults r = ifc.decompileFunction(f, 60, monitor);
                w.println("// ==== " + f.getName() + " @ " + f.getEntryPoint() + " ====");
                if (r != null && r.decompileCompleted()) { w.println(r.getDecompiledFunction().getC()); ok++; }
                else { w.println("// decompile failed: " + (r == null ? "null" : r.getErrorMessage())); fail++; }
            }
        }
        ifc.dispose();
        println("DecompileAll: " + ok + " ok, " + fail + " failed -> " + out.getAbsolutePath());
    }
}
