import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const version = (process.argv[2] ?? "").replace(/^v/, "");
if (!/^\d+\.\d+\.\d+$/.test(version)) {
  console.error("usage: npm run release -- 0.2.0");
  process.exit(1);
}

const git = (...args) => execFileSync("git", args, { cwd: root, encoding: "utf8" }).trim();
if (git("status", "--porcelain")) {
  console.error("Commit or discard your changes first.");
  process.exit(1);
}

const edit = (file, fn) => {
  const path = join(root, file);
  writeFileSync(path, fn(readFileSync(path, "utf8")));
};
const setJson = (file) =>
  edit(file, (text) => JSON.stringify({ ...JSON.parse(text), version }, null, 2) + "\n");
const setToml = (file) => edit(file, (text) => text.replace(/^version = ".*"$/m, `version = "${version}"`));

setJson("package.json");
setJson("src-tauri/tauri.conf.json");
setToml("src-tauri/Cargo.toml");
setToml("crates/pulse-engine/Cargo.toml");
setToml("crates/pulse-host/Cargo.toml");
execFileSync(process.platform === "win32" ? "cargo.exe" : "cargo", ["update", "--workspace", "--offline"], {
  cwd: root,
  stdio: "inherit",
});

git("add", "-A");
git("commit", "-m", `Release v${version}`);
git("tag", "-a", `v${version}`, "-m", `Pulse DM v${version}`);
console.log(`Tagged v${version}. Push the commit and the tag to publish it:\n  git push --follow-tags`);
