// Downloads the signed XPI of a version that is already on AMO, for re-runs of a release
// (AMO refuses to take the same version twice). Uses WEB_EXT_API_KEY / WEB_EXT_API_SECRET.
import { createHmac, randomUUID } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const manifest = JSON.parse(readFileSync(join(root, "target", "firefox-extension", "manifest.json"), "utf8"));
const guid = manifest.browser_specific_settings.gecko.id;
const version = manifest.version;
const out = join(root, "target", "xpi");

const b64 = (s) => Buffer.from(s).toString("base64url");
function auth() {
  const now = Math.floor(Date.now() / 1000);
  const body = `${b64(JSON.stringify({ alg: "HS256", typ: "JWT" }))}.${b64(
    JSON.stringify({ iss: process.env.WEB_EXT_API_KEY, jti: randomUUID(), iat: now, exp: now + 60 }),
  )}`;
  const sig = createHmac("sha256", process.env.WEB_EXT_API_SECRET).update(body).digest("base64url");
  return { Authorization: `JWT ${body}.${sig}` };
}

async function findFile() {
  let next = `https://addons.mozilla.org/api/v5/addons/addon/${encodeURIComponent(guid)}/versions/?filter=all_with_unlisted&page_size=50`;
  while (next) {
    const res = await fetch(next, { headers: auth() });
    if (!res.ok) throw new Error(`AMO versions: ${res.status} ${await res.text()}`);
    const page = await res.json();
    const hit = page.results.find((v) => v.version === version);
    if (hit) return hit.file;
    next = page.next;
  }
  throw new Error(`version ${version} of ${guid} is not on AMO`);
}

const deadline = Date.now() + 15 * 60 * 1000;
let file = await findFile();
while (file.status !== "public") {
  if (file.status === "disabled" || Date.now() > deadline) throw new Error(`version ${version} is still "${file.status}" on AMO`);
  console.log(`Waiting for AMO to sign ${version} (status: ${file.status})...`);
  await new Promise((r) => setTimeout(r, 15000));
  file = await findFile();
}

const res = await fetch(file.url, { headers: auth() });
if (!res.ok) throw new Error(`download ${file.url}: ${res.status}`);
mkdirSync(out, { recursive: true });
const dest = join(out, `pulse_dm-${version}.xpi`);
writeFileSync(dest, Buffer.from(await res.arrayBuffer()));
console.log(`Signed ${version} from AMO -> ${dest}`);
