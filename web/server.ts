import indexHtml from "./index.html";

const port = Number(Bun.env.PORT || 3000);

const server = Bun.serve({
  port,
  routes: {
    "/": indexHtml,
    "/public/rmcl.svg": () =>
      new Response(Bun.file(new URL("./public/rmcl.svg", import.meta.url)), {
        headers: { "Content-Type": "image/svg+xml; charset=utf-8" },
      }),
    "/public/screenshots/:file": (req) => {
      const file = req.params.file;
      if (!/^[a-z0-9_-]+\.(webp|png)$/i.test(file)) {
        return new Response("Not Found", { status: 404 });
      }
      const contentType = file.endsWith(".webp") ? "image/webp" : "image/png";
      return new Response(
        Bun.file(new URL(`./public/screenshots/${file}`, import.meta.url)),
        {
          headers: {
            "Content-Type": contentType,
            "Cache-Control": "public, max-age=3600",
          },
        },
      );
    },
  },
  development: Bun.env.NODE_ENV !== "production" && {
    hmr: true,
    console: true,
  },
});

console.log(`RMCL Landing Page running at ${server.url}`);
