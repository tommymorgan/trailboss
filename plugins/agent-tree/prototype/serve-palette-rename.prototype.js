// THROWAWAY fixture-only preview. Does not connect to a Core or mutate a session.
// Run: npm exec --yes --package=bun -- bun plugins/agent-tree/prototype/serve-palette-rename.prototype.js
if (process.env.NODE_ENV === 'production') throw new Error('Prototype is development-only');
const build = await Bun.build({
  entrypoints: [new URL('../project.ts', import.meta.url).pathname],
  target: 'browser',
});
if (!build.success) throw new Error(String(build.logs));
const routes = new Map([
  ['/', 'palette-rename.prototype.html'],
  ['/palette-rename.prototype.html', 'palette-rename.prototype.html'],
  ['/sidebar.projection.prototype.js', 'sidebar.projection.prototype.js'],
]);
const server = Bun.serve({
  hostname: '0.0.0.0',
  port: Number(process.env.PORT ?? 8892),
  async fetch(request) {
    const path = new URL(request.url).pathname;
    if (path === '/production-sidebar.prototype.js') {
      return new Response(build.outputs[0], {
        headers: { 'Content-Type': 'text/javascript', 'Cache-Control': 'no-store' },
      });
    }
    const filename = routes.get(path);
    if (!filename) return new Response('Not found', { status: 404 });
    return new Response(Bun.file(new URL(filename, import.meta.url)), {
      headers: { 'Cache-Control': 'no-store' },
    });
  },
});
console.log(`Palette rename prototype ready on port ${server.port}`);
