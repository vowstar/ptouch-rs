// Map MSVC RTTI class names -> vtables -> ESC i command constants reachable from their methods.
// Usage: node rtti_map.mjs <brdvset.exe> <brdvset.exe.c> [classFilter]
import fs from "node:fs";

const [exePath, cPath, filter = "Item"] = process.argv.slice(2);
const b = fs.readFileSync(exePath);
const pe = b.readUInt32LE(0x3c);
const nsec = b.readUInt16LE(pe + 6);
const optSz = b.readUInt16LE(pe + 20);
const base = b.readUInt32LE(pe + 24 + 28);
const secs = [];
for (let i = 0; i < nsec; i++) {
  const o = pe + 24 + optSz + 40 * i;
  secs.push({
    name: b.toString("latin1", o, o + 8).replace(/\0.*$/, ""),
    va: base + b.readUInt32LE(o + 12), vsz: b.readUInt32LE(o + 8),
    raw: b.readUInt32LE(o + 20), rsz: b.readUInt32LE(o + 16),
  });
}
const text = secs.find((s) => s.name === ".text");
const off2va = (off) => { for (const s of secs) if (off >= s.raw && off < s.raw + s.rsz) return s.va + off - s.raw; return null; };
const va2off = (va) => { for (const s of secs) if (va >= s.va && va < s.va + s.rsz) return s.raw + va - s.va; return null; };
const inText = (va) => va >= text.va && va < text.va + text.vsz;

// Index every aligned dword -> offsets where it occurs.
const dwordAt = new Map();
for (let i = 0; i + 4 <= b.length; i += 4) {
  const v = b.readUInt32LE(i);
  if (!dwordAt.has(v)) dwordAt.set(v, []);
  dwordAt.get(v).push(i);
}

// Decompiled C: function -> body text.
const src = fs.readFileSync(cPath, "utf8");
const funcs = new Map();
const re = /^\/\/ ==== (\S+) @ ([0-9a-f]+) ====$/gm;
let m, prev = null;
while ((m = re.exec(src))) {
  if (prev) funcs.get(prev.addr).body = src.slice(prev.start, m.index);
  const addr = parseInt(m[2], 16);
  funcs.set(addr, { name: m[1], body: "" });
  prev = { addr, start: m.index };
}
if (prev) funcs.get(prev.addr).body = src.slice(prev.start);

const escIn = (body) => [...new Set((body.match(/0x[0-9a-f]{0,4}691b\b/gi) || []))]
  .map((c) => { const v = parseInt(c, 16) >>> 0; const bytes = [v & 0xff, (v >> 8) & 0xff, (v >> 16) & 0xff, (v >>> 24) & 0xff];
    return bytes.map((x) => x.toString(16).padStart(2, "0")).join(" ") + ` ("ESC i ${String.fromCharCode(bytes[2])}${bytes[3] >= 0x20 && bytes[3] < 0x7f ? " " + String.fromCharCode(bytes[3]) : ""}")`; });
const callees = (body) => [...new Set((body.match(/FUN_[0-9a-f]{8}/g) || []))].map((n) => parseInt(n.slice(4), 16));

function reach(addr, depth) {
  const seen = new Set(); const out = new Set();
  const walk = (a, d) => { if (seen.has(a)) return; seen.add(a); const f = funcs.get(a); if (!f) return;
    escIn(f.body).forEach((e) => out.add(e)); if (d > 0) callees(f.body).forEach((c) => walk(c, d - 1)); };
  walk(addr, depth);
  return [...out];
}

// RTTI: TypeDescriptor name at TD+8; COL (x86) = {sig=0, off, cdOff, pTD, pCHD}; vtable[-1] = &COL.
const results = [];
for (let i = 0; i < b.length - 4; i++) {
  if (b[i] !== 0x2e || b[i + 1] !== 0x3f || b[i + 2] !== 0x41 || b[i + 3] !== 0x56) continue; // ".?AV"
  const end = b.indexOf(0, i);
  const name = b.toString("latin1", i, end);
  if (!/^\.\?AV\w+@@$/.test(name) || !new RegExp(filter).test(name)) continue;
  const tdVa = off2va(i - 8);
  for (const tdRef of dwordAt.get(tdVa) || []) {
    const colOff = tdRef - 12;
    if (colOff < 0 || b.readUInt32LE(colOff) !== 0 || b.readUInt32LE(colOff + 4) !== 0) continue; // primary vtable only
    const colVa = off2va(colOff);
    for (const colRef of dwordAt.get(colVa) || []) {
      const vtVa = off2va(colRef + 4);
      const slots = [];
      for (let k = 0; k < 40; k++) { const v = b.readUInt32LE(colRef + 4 + 4 * k); if (!inText(v)) break; slots.push(v); }
      const cmds = new Set(); const perSlot = [];
      slots.forEach((s, k) => { const r = reach(s, 2); r.forEach((c) => cmds.add(c)); if (r.length) perSlot.push(`slot${k}@${s.toString(16)}: ${r.join(", ")}`); });
      results.push({ cls: name.slice(4, -2), vtable: vtVa.toString(16), slots: slots.length, cmds: [...cmds], perSlot });
    }
  }
}
results.sort((a, b2) => a.cls.localeCompare(b2.cls));
for (const r of results) {
  console.log(`${r.cls.padEnd(30)} vtbl=${r.vtable} slots=${r.slots}  ${r.cmds.join(" | ") || "-"}`);
  if (process.env.VERBOSE) r.perSlot.forEach((p) => console.log("      " + p));
}
