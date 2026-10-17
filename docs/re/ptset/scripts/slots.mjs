// Print decompiled bodies of chosen vtable slots. Usage: node slots.mjs <exe> <c> <vtableHex> <slot,slot,...>
import fs from "node:fs";
const [exePath, cPath, vtHex, slotList] = process.argv.slice(2);
const b = fs.readFileSync(exePath);
const pe = b.readUInt32LE(0x3c), nsec = b.readUInt16LE(pe + 6), optSz = b.readUInt16LE(pe + 20), base = b.readUInt32LE(pe + 52);
const secs = [...Array(nsec)].map((_, i) => { const o = pe + 24 + optSz + 40 * i;
  return { va: base + b.readUInt32LE(o + 12), raw: b.readUInt32LE(o + 20), rsz: b.readUInt32LE(o + 16) }; });
const va2off = (va) => { for (const s of secs) if (va >= s.va && va < s.va + s.rsz) return s.raw + va - s.va; };
const src = fs.readFileSync(cPath, "utf8");
const body = (addr) => { const h = `// ==== `; const tag = ` @ ${addr.toString(16).padStart(8, "0")} ====`;
  const i = src.indexOf(tag); if (i < 0) return `// (no decompiled body for ${addr.toString(16)})`;
  const start = src.lastIndexOf(h, i); const next = src.indexOf("\n// ==== ", i + tag.length); return src.slice(start, next < 0 ? undefined : next).trim(); };
const vt = parseInt(vtHex, 16);
for (const s of slotList.split(",").map(Number)) {
  const fn = b.readUInt32LE(va2off(vt + 4 * s));
  console.log(`\n######## slot ${s} -> ${fn.toString(16)}`);
  console.log(body(fn).replace(/\n\s*\n/g, "\n"));
}
