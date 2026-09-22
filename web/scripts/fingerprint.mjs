// Fingerprint web/src so the offline Rust suite can catch a stale bundle.
//
// Runs last in `npm run build` (after vite wipes dist/) and writes
// dist/.src-hash: the FNV-1a 64 hex of every file under web/src, fed as
// relpath bytes, one 0x00 byte, then file bytes with CRLF normalized to LF
// (so Windows checkouts hash the same), in sorted path order.
// tests/stats.rs recomputes the same digest: any src change without a
// rebuild fails the suite with no node involved.
import { readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

const web = join(fileURLToPath(import.meta.url), "..", "..");
const src = join(web, "src");

function walk(dir) {
  const found = [];
  for (const entry of readdirSync(dir).sort()) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) {
      found.push(...walk(full));
    } else {
      found.push(relative(web, full).split(sep).join("/"));
    }
  }
  return found;
}

// FNV-1a 64, matching the Rust twin in tests/stats.rs.
let digest = 14695981039346656037n;
const prime = 1099511628211n;
const mask = 0xffffffffffffffffn;
function feed(bytes) {
  for (const byte of bytes) {
    digest = ((digest ^ BigInt(byte)) * prime) & mask;
  }
}

const encoder = new TextEncoder();
const files = walk(src).sort();
for (const file of files) {
  feed(encoder.encode(file));
  feed([0]);
  const bytes = readFileSync(join(web, file));
  for (let i = 0; i < bytes.length; i++) {
    // A Windows checkout stores CRLF; the hash sees LF either way.
    if (bytes[i] === 0x0d && bytes[i + 1] === 0x0a) continue;
    feed([bytes[i]]);
  }
}

const out = join(web, "dist", ".src-hash");
writeFileSync(out, digest.toString(16).padStart(16, "0") + "\n");
console.log(`fingerprinted ${files.length} files -> dist/.src-hash`);
