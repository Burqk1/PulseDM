import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const target = process.env.CARGO_TARGET_DIR || join(root, "target");
const LIMIT_GB = Number(process.env.PULSE_TARGET_LIMIT_GB) || 10;

function sizeOf(path) {
  const st = statSync(path, { throwIfNoEntry: false });
  if (!st) return 0;
  if (!st.isDirectory()) return st.size;
  return readdirSync(path).reduce((sum, name) => sum + sizeOf(join(path, name)), 0);
}

if (existsSync(target)) {
  const gb = sizeOf(target) / 1024 ** 3;
  if (gb > LIMIT_GB || process.argv.includes("--force")) {
    console.log(`sweep: target is ${gb.toFixed(1)} GB (limit ${LIMIT_GB} GB), running cargo clean`);
    execFileSync(process.platform === "win32" ? "cargo.exe" : "cargo", ["clean"], { cwd: root, stdio: "inherit" });
  }
}
