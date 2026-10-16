import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const cargo = process.platform === "win32" ? "cargo.exe" : "cargo";

await import("./sweep.mjs").catch((e) => console.warn(`sweep skipped: ${e.message}`));

const triple = /host: (\S+)/.exec(execFileSync("rustc", ["-vV"], { encoding: "utf8" }))[1];
execFileSync(cargo, ["build", "--release", "-p", "pulse-host"], { cwd: root, stdio: "inherit" });

const ext = process.platform === "win32" ? ".exe" : "";
const out = join(root, "src-tauri", "binaries", `pulsedm-host-${triple}${ext}`);
mkdirSync(dirname(out), { recursive: true });
copyFileSync(join(root, "target", "release", `pulsedm-host${ext}`), out);
console.log(`pulsedm-host -> ${out}`);
