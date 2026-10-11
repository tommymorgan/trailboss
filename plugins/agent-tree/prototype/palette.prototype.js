// THROWAWAY: production sidebar with a centered keyboard action palette.
import { mountProductionRows } from './sidebar.projection.prototype.js';
export function mountPalette(root, state, api) {
  const doc = root.ownerDocument;
  const style = doc.createElement('style');
  style.textContent = `
    .at-palette-overlay{font:12px/1.4 ui-monospace,monospace;color:#d7d1df;box-sizing:border-box}
    .at-palette-overlay{position:fixed;inset:0;z-index:1000;display:grid;place-items:center;background:#08060cb8;padding:20px}
    .at-palette-overlay .ap-dialog{width:min(480px,100%);background:#19131f;border:1px solid #574367;border-radius:9px;box-shadow:0 24px 90px #0009;overflow:hidden}
    .at-palette-overlay .ap-title{margin:0;padding:16px 16px 5px;font-size:14px;color:#f0e4fb;overflow-wrap:anywhere}
    .at-palette-overlay .ap-description{margin:0;padding:0 16px 12px;font-size:11px;color:#a591b6}
    .at-palette-overlay .ap-input{display:block;box-sizing:border-box;width:calc(100% - 24px);margin:0 12px 10px;padding:11px;border:1px solid #5e4772;border-radius:4px;background:#100c16;color:#f0e4fb;font:inherit;outline:none}
    .at-palette-overlay .ap-input:focus{border-color:#ba91dc}
    .at-palette-overlay .ap-results{max-height:300px;overflow:auto;padding:0 7px 7px}
    .at-palette-overlay .ap-result{display:flex;align-items:center;justify-content:space-between;gap:12px;text-align:left;width:100%;border:0;border-radius:4px;background:transparent;padding:10px;color:#d8cbdf}
    .at-palette-overlay .ap-result[aria-selected=true],.at-palette-overlay .ap-result:hover{background:#382843;color:#fff0ff}
    .at-palette-overlay .ap-result:focus-visible{outline:1px solid #ba91dc;outline-offset:-1px}
    .at-palette-overlay .ap-detail{color:#ad97bb;font-size:10px;flex:none}
    .at-palette-overlay .ap-empty{padding:20px;color:#aa96b8;text-align:center}
    .at-palette-overlay .ap-footer{border-top:1px solid #35283f;padding:10px 16px;color:#9885a7;font-size:10px}
  `;
  doc.head.append(style);
  const sidebar = doc.createElement('nav');
  sidebar.className = 'at-palette-sidebar';
  sidebar.setAttribute('aria-label', 'Spaces and agents — keyboard palette');
  root.replaceChildren(sidebar);
  let scopeId = state.spaces.find(space => space.id === state.selectedSpace)?.id ?? null;
  let overlay = null;
  let input = null;
  let results = null;
  let commands = [];
  let filtered = [];
  let active = 0;
  let previousFocus = null;


  function element(tag, className, text) {
    const node = doc.createElement(tag);
    node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  }
  function setScope(id) {
    scopeId = id;

  }
  function dismiss(restore = true) {
    overlay?.remove();
    overlay = null;
    input = null;
    results = null;
    if (restore && previousFocus?.isConnected) previousFocus.focus();
  }
  function run(name, spaceId, tabId) {
    dismiss();
    api.action(name, spaceId, tabId);
  }
  function activate(index) {
    filtered[index]?.run();
  }
  function highlight(index) {
    if (!filtered.length) return;
    active = (index + filtered.length) % filtered.length;
    [...results.children].forEach((node, i) => node.setAttribute('aria-selected', String(i === active)));
    input.setAttribute('aria-activedescendant', results.children[active].id);
    results.children[active].scrollIntoView({ block: 'nearest' });
  }
  function filter() {
    const query = input.value.trim().toLocaleLowerCase();
    filtered = commands.filter(command => `${command.label} ${command.detail}`.toLocaleLowerCase().includes(query));
    active = 0;
    results.replaceChildren();
    input.removeAttribute('aria-activedescendant');
    filtered.forEach((command, index) => {
      const button = element('button', 'ap-result');
      button.type = 'button';
      button.id = `${results.id}-${index}`;
      button.setAttribute('role', 'option');
      button.setAttribute('aria-selected', String(index === active));
      button.append(element('span', '', command.label), element('span', 'ap-detail', command.detail));
      button.addEventListener('click', () => activate(index));
      button.addEventListener('focus', () => highlight(index));
      results.append(button);
    });
    if (!filtered.length) results.append(element('div', 'ap-empty', 'No matching actions'));
    else input.setAttribute('aria-activedescendant', results.children[0].id);
  }
  function open(space = null) {
    const alreadyOpen = Boolean(overlay);
    if (!alreadyOpen) previousFocus = doc.activeElement;
    dismiss(false);
    overlay = element('div', 'at-palette-overlay');
    const dialog = element('section', 'ap-dialog');
    const unique = `ap-${Math.random().toString(36).slice(2)}`;
    dialog.setAttribute('role', 'dialog');
    dialog.setAttribute('aria-modal', 'true');
    dialog.setAttribute('aria-labelledby', `${unique}-title`);
    const title = element('h2', 'ap-title', space ? `Actions · ${space.name}` : 'Session actions');
    title.id = `${unique}-title`;
    const description = element('p', 'ap-description', space ? `All tab actions apply to “${space.name}”.` : 'Choose a space to create or manage its tabs.');
    input = element('input', 'ap-input');
    input.type = 'text';
    input.placeholder = space ? `Filter actions in ${space.name}…` : 'Find a session action or space…';
    input.setAttribute('aria-label', 'Filter palette actions');
    input.setAttribute('role', 'combobox');
    input.setAttribute('aria-expanded', 'true');
    input.setAttribute('aria-autocomplete', 'list');
    results = element('div', 'ap-results');
    results.id = `${unique}-results`;
    results.setAttribute('role', 'listbox');
    results.setAttribute('aria-label', space ? `${space.name} actions` : 'Session actions and spaces');
    input.setAttribute('aria-controls', results.id);
    commands = space ? [
      { label: 'New tab', detail: space.name, run: () => run('new-tab', space.id) },
      { label: 'Rename space', detail: space.name, run: () => run('rename-space', space.id) },
      ...space.tabs.map(tab => ({ label: `Close tab · ${tab.name}`, detail: space.name, run: () => run('close-tab', space.id, tab.id) })),
    ] : [
      { label: 'New space', detail: 'Session', run: () => run('new-space') },
      { label: 'Detach', detail: 'Session', run: () => run('detach') },
      ...state.spaces.map(item => ({ label: item.name, detail: 'Space actions →', run: () => { setScope(item.id); open(item); } })),
    ];
    input.addEventListener('input', filter);
    dialog.append(title, description, input, results, element('div', 'ap-footer', '↑ ↓ Navigate · Enter Run · Esc Dismiss · Ctrl+K Session'));
    overlay.append(dialog);
    overlay.addEventListener('click', event => { if (event.target === overlay) dismiss(); });
    doc.body.append(overlay);
    filter();
    input.focus();
  }

  const tree = mountProductionRows(sidebar, state, api, ({button,space}) => {
    button.addEventListener('focus', () => setScope(space.id));
  });

  function keydown(event) {
    if (event.ctrlKey && !event.altKey && event.key.toLowerCase() === 'k') {
      event.preventDefault();
      event.stopPropagation();
      open();
      return;
    }
    if (event.shiftKey && event.key === 'F10' && !overlay) {
      const space = state.spaces.find(item => item.id === scopeId);
      if (!space) return;
      event.preventDefault();
      event.stopPropagation();
      open(space);
      return;
    }
    if (overlay) {
      if (event.key === 'Escape') {
        event.preventDefault();
        event.stopPropagation();
        dismiss();
      } else if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
        event.preventDefault();
        event.stopPropagation();
        highlight(active + (event.key === 'ArrowDown' ? 1 : -1));
      } else if (event.key === 'Enter') {
        event.preventDefault();
        event.stopPropagation();
        activate(active);
      } else if (event.key === 'Tab') {
        event.preventDefault();
        const focusable = [input, ...results.querySelectorAll('button')];
        const index = focusable.indexOf(doc.activeElement);
        focusable[(index + (event.shiftKey ? -1 : 1) + focusable.length) % focusable.length].focus();
      }
      return;
    }
    if (sidebar.contains(event.target) && ['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) {
      const rows = [...tree.querySelectorAll('button')];
      if (!rows.length) return;
      const index = rows.indexOf(doc.activeElement);
      const next = event.key === 'Home' ? 0 : event.key === 'End' ? rows.length - 1 : (index + (event.key === 'ArrowDown' ? 1 : -1) + rows.length) % rows.length;
      event.preventDefault();
      event.stopPropagation();
      rows[next].focus();
    }
  }
  doc.addEventListener('keydown', keydown, true);
  return () => {
    doc.removeEventListener('keydown', keydown, true);
    dismiss(false);
    style.remove();
    sidebar.remove();
  };
}
