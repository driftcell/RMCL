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
  },
  development: Bun.env.NODE_ENV !== "production" && {
    hmr: true,
    console: true,
  },
});

console.log(`RMCL Landing Page running at ${server.url}`);
