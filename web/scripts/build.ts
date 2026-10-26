import { cp, rm } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const dist = join(root, "dist");

await rm(dist, { recursive: true, force: true });

const build = Bun.spawnSync(
  ["bun", "build", "./index.html", "--outdir", "./dist", "--minify"],
  {
    cwd: root,
    stdout: "inherit",
    stderr: "inherit",
  },
);

if (build.exitCode !== 0) {
  process.exit(build.exitCode ?? 1);
}

for (const file of ["robots.txt", "sitemap.xml", "og-image.png"]) {
  await cp(join(root, "public", file), join(dist, file));
}
