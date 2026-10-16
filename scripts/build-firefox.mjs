import { cpSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const src = join(root, "extension");
const out = join(root, "target", "firefox-extension");

rmSync(out, { recursive: true, force: true });
cpSync(src, out, { recursive: true });

const shared = readFileSync(join(src, "shared.js"), "utf8").replace(/^export /gm, "");
const background = readFileSync(join(src, "background.js"), "utf8").replace(/^import .* from "\.\/shared\.js";\r?\n/m, "");
writeFileSync(join(out, "background.js"), `${shared}\n${background}`);

const manifest = JSON.parse(readFileSync(join(src, "manifest.json"), "utf8"));
delete manifest.key;
manifest.manifest_version = 2;
manifest.version = JSON.parse(readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8")).version;
manifest.permissions = [...manifest.permissions, "webRequestBlocking", ...(manifest.host_permissions ?? [])];
delete manifest.host_permissions;
manifest.background = { scripts: ["background.js"] };
manifest.browser_action = manifest.action;
delete manifest.action;
manifest.browser_specific_settings = {
  gecko: { id: "pulsedm@burqk1", strict_min_version: "142.0", data_collection_permissions: { required: ["none"] } },
};
writeFileSync(join(out, "manifest.json"), JSON.stringify(manifest, null, 2) + "\n");
console.log(`Firefox extension -> ${out}`);
