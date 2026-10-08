import { build } from "esbuild";
import { readFile, writeFile, readdir } from "node:fs/promises";
import path from "node:path";
import { createHash } from "node:crypto";

const result = await build({
  entryPoints: ["studio_ui/editor.js"],
  bundle: true,
  minify: true,
  format: "iife",
  target: "chrome100",
  outfile: "studio_ui/vendor/editor.bundle.js",
  legalComments: "linked",
  metafile: true,
});
const packages = new Set();
for (const input of Object.keys(result.metafile.inputs)) {
  const parts = input.replaceAll("\\", "/").split("/");
  const index = parts.lastIndexOf("node_modules");
  if (index >= 0)
    packages.add(
      parts
        .slice(0, index + (parts[index + 1].startsWith("@") ? 3 : 2))
        .join("/"),
    );
}
const notices = [];
for (const directory of [...packages].sort()) {
  const metadata = JSON.parse(
    await readFile(path.join(directory, "package.json"), "utf8"),
  );
  const file = (await readdir(directory)).find((name) =>
    /^licen[cs]e(?:\.|$)/i.test(name),
  );
  if (!file) throw new Error("Missing license: " + metadata.name);
  notices.push(
    `${metadata.name} ${metadata.version}\n${await readFile(path.join(directory, file), "utf8")}`,
  );
}
await writeFile(
  "studio_ui/vendor/EDITOR-LICENSES.txt",
  notices.join("\n\n────────────────────────────\n\n"),
);
console.log(
  "Bundled CodeMirror and included licenses for",
  packages.size,
  "packages.",
);

let html = await readFile("studio_ui/index.html", "utf8");
for (const asset of [
  "style.css",
  "app.js",
  "workspace.js",
  "updates.js",
  "vendor/editor.bundle.js",
]) {
  const hash = createHash("sha256")
    .update(await readFile(`studio_ui/${asset}`))
    .digest("hex")
    .slice(0, 12);
  const escaped = asset.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  html = html.replace(
    new RegExp(`(/static/${escaped})(?:\\?v=[^"']*)?`, "g"),
    `$1?v=${hash}`,
  );
}
await writeFile("studio_ui/index.html", html);
