// THROWAWAY preview server. Run: bun plugins/agent-tree/prototype/serve.prototype.js
if (process.env.NODE_ENV === 'production') throw new Error('Prototype is development-only');
const build = await Bun.build({entrypoints:[new URL('../project.ts', import.meta.url).pathname],target:'browser'});
if (!build.success) throw new Error(String(build.logs));
const routes = new Map([
  ['/', 'sidebar-menus.prototype.html'],
  ['/sidebar-menus.prototype.html', 'sidebar-menus.prototype.html'],
  ['/context-hover.prototype.js', 'context-hover.prototype.js'],
  ['/palette.prototype.js', 'palette.prototype.js'],
  ['/sidebar.projection.prototype.js', 'sidebar.projection.prototype.js'],
]);
Bun.serve({
  hostname: '0.0.0.0', port: 8891,
  async fetch(request) {
    const path = new URL(request.url).pathname;
    if(path === '/production-sidebar.prototype.js') return new Response(build.outputs[0], {headers:{'Content-Type':'text/javascript','Cache-Control':'no-store'}});
    const filename = routes.get(path);
    if (!filename) return new Response('Not found', { status: 404 });
    return new Response(Bun.file(new URL(filename, import.meta.url)), { headers: { 'Cache-Control': 'no-store' } });
  },
});
console.log('Sidebar menu prototype: http://192.168.1.133:8891/?variant=b');
