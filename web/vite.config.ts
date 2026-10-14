import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig, type Plugin } from "vite";

const here = path.dirname(fileURLToPath(import.meta.url));

/** Ship the license of every npm package that ends up in the bundle. */
function thirdPartyLicenses(): Plugin {
  return {
    name: "studio-third-party-licenses",
    apply: "build",
    generateBundle() {
      const packages = new Set<string>();
      for (const id of this.getModuleIds()) {
        const parts = id.replaceAll("\\", "/").split("/");
        const index = parts.lastIndexOf("node_modules");
        if (index < 0 || !parts[index + 1]) continue;
        const depth = parts[index + 1].startsWith("@") ? 3 : 2;
        packages.add(parts.slice(0, index + depth).join("/"));
      }
      const notices = [
        "Manim Studio's interface adapts the design of T3 Code (https://github.com/pingdotgg/t3code).\n" +
          readFileSync(path.join(here, "T3CODE-LICENSE.txt"), "utf8"),
      ];
      for (const directory of [...packages].sort()) {
        const metadata = JSON.parse(
          readFileSync(path.join(directory, "package.json"), "utf8"),
        );
        const file = readdirSync(directory).find((name) =>
          /^licen[cs]e(?:\.|$)/i.test(name),
        );
        if (!file) throw new Error(`Missing license for ${metadata.name}`);
        notices.push(
          `${metadata.name} ${metadata.version}\n${readFileSync(path.join(directory, file), "utf8")}`,
        );
      }
      this.emitFile({
        type: "asset",
        fileName: "THIRD-PARTY-LICENSES.txt",
        source: notices.join("\n\n────────────────────────────\n\n"),
      });
    },
  };
}

export default defineConfig({
  root: here,
  base: "/static/",
  publicDir: "public",
  plugins: [react(), tailwindcss(), thirdPartyLicenses()],
  resolve: { alias: { "~": path.join(here, "src") } },
  build: {
    outDir: path.join(here, "..", "studio_ui"),
    emptyOutDir: true,
    target: "chrome110",
    chunkSizeWarningLimit: 1500,
  },
});
