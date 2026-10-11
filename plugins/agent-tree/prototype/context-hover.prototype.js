// THROWAWAY: identical production sidebar; compare right-click and reveal-only menus.
import { mountProductionRows } from './sidebar.projection.prototype.js';
// The host owns state, mutations, full rerenders, and the variant switcher.
const CSS = `
.chp-menu{box-sizing:border-box;font:13px/1.4 ui-monospace,monospace;color:#b9b3bd;position:fixed;z-index:1000;width:244px;max-width:calc(100vw - 16px);background:#211926;border:1px solid #49364e;border-radius:7px;padding:5px;box-shadow:0 12px 36px #0009}
.chp-menu *{box-sizing:border-box}
.chp-trigger{visibility:hidden;opacity:0}
.chp-empty{flex:1;min-height:22px}
.chp-menu.chp-attached{border-top:2px solid #6f8fc0}
.chp-menu-title{padding:5px 7px 7px;color:#93859f;font-size:10px;overflow-wrap:anywhere;border-bottom:1px solid #ffffff0b;margin-bottom:3px}
.chp-menu button{font:inherit;color:inherit;display:block;width:100%;padding:7px;background:transparent;text-align:left;border:0;border-radius:3px;overflow-wrap:anywhere;cursor:pointer}
.chp-menu button:hover,.chp-menu button:focus{background:#3a2b43}
.chp-menu .chp-danger{color:#df8c88}
`;

function element(doc, tag, className, text) {
  const node = doc.createElement(tag);
  node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function mount(root, state, api, hover) {
  const doc = root.ownerDocument;
  const win = doc.defaultView;
  const style = element(doc, 'style', '');
  style.textContent = CSS;
  const sidebar = element(doc, 'nav', `chp-sidebar ${hover ? 'chp-hover' : 'chp-context'}`);
  sidebar.setAttribute('aria-label', hover ? 'Space sections — hover or focus for actions' : 'Spaces — right-click or Shift+F10 for actions');
  sidebar.tabIndex = 0;
  root.replaceChildren(style, sidebar);
  let menu = null;
  let opener = null;

  function dismiss(restore = false) {
    if (!menu) return;
    menu.remove();
    menu = null;
    opener?.setAttribute('aria-expanded', 'false');
    if (restore && opener?.isConnected) opener.focus();
    opener = null;
  }

  function showMenu(scope, source, point) {
    dismiss();
    opener = source;
    source.setAttribute('aria-expanded', 'true');
    const { space, tab } = scope;
    const actions = tab
      ? [{ label: `Close tab “${tab.name}”`, name: 'close-tab', danger: true }]
      : space
        ? [{ label: `New tab in “${space.name}”`, name: 'new-tab' }, { label: `Rename “${space.name}”`, name: 'rename-space' }]
        : [{ label: 'New space', name: 'new-space' }, { label: 'Detach from session', name: 'detach' }];
    menu = element(doc, 'div', `chp-menu${hover ? ' chp-attached' : ''}`);
    menu.setAttribute('role', 'menu');
    menu.setAttribute('aria-label', tab ? `Tab actions: ${tab.name}` : space ? `Space actions: ${space.name}` : 'Session actions');
    if (tab || !space) menu.append(element(doc, 'div', 'chp-menu-title', tab ? `${space.name} / ${tab.name}` : 'SESSION · all spaces'));
    for (const action of actions) {
      const button = element(doc, 'button', action.danger ? 'chp-danger' : '', action.label);
      button.type = 'button';
      button.setAttribute('role', 'menuitem');
      button.addEventListener('click', () => {
        dismiss();
        api.action(action.name, space?.id, tab?.id);
      });
      menu.append(button);
    }
    doc.body.append(menu);
    const rect = source.getBoundingClientRect();
    const x = point ? point.x : hover ? rect.left : rect.left + 12;
    const y = point ? point.y : rect.bottom + 3;
    const bounds = menu.getBoundingClientRect();
    menu.style.left = `${Math.max(8, Math.min(x, win.innerWidth - bounds.width - 8))}px`;
    menu.style.top = `${Math.max(8, Math.min(y, win.innerHeight - bounds.height - 8))}px`;
    menu.querySelector('button').focus();
  }

  function bindContext(node, scope, source = node) {
    node.addEventListener('contextmenu', event => {
      event.preventDefault();
      event.stopPropagation();
      showMenu(scope, source, { x: event.clientX, y: event.clientY });
    });
    node.addEventListener('keydown', event => {
      if (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10')) {
        event.preventDefault();
        event.stopPropagation();
        showMenu(scope, source);
      }
    });
  }

  // Sidebar labels, order, roles and selection are provided by the shared production adapter.

  function trigger(line, scope, label) {
    const button = element(doc, 'button', 'chp-trigger', '⋯');
    button.type = 'button';
    button.setAttribute('aria-label', label);
    button.setAttribute('aria-haspopup', 'menu');
    button.setAttribute('aria-expanded', 'false');
    // Revealing on row focus makes the otherwise-hidden trigger tabbable next.
    button.addEventListener('click', () => {
      if (menu && opener === button) dismiss(true);
      else showMenu(scope, button);
    });
    line.append(button);
  }

  mountProductionRows(sidebar, state, api, ({line,button,space,tab,pane,section}) => {
    const scope={space, ...(tab ? {tab} : {})};
    if (!hover) bindContext(line, scope, button);
    else if (!pane) trigger(line, scope, section ? `Actions for space ${space.name}` : `Actions for tab ${tab.name} in ${space.name}`);
  });
  const empty = element(doc, 'div', 'chp-empty');
  sidebar.append(empty);
  if (hover) {
    const footer = element(doc, 'div', 'production-row production-footer');
    footer.tabIndex = 0;
    footer.setAttribute('aria-label', 'Session actions — focus to reveal menu');
    trigger(footer, {}, 'Session actions');
    footer.addEventListener('keydown', event => {
      if (event.target === footer && (event.key === 'Enter' || event.key === ' ')) {
        event.preventDefault();
        showMenu({}, footer.querySelector('button'));
      }
    });
    sidebar.append(footer);
  } else {
    bindContext(sidebar, {});
    // Instructions belong to the prototype host, not the production sidebar.
  }

  const outside = event => {
    if (menu && !menu.contains(event.target) && !opener?.contains(event.target)) dismiss();
  };
  const keyboard = event => {
    if (!menu) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      dismiss(true);
    } else if (event.key === 'Tab') {
      // Return to the launching row before native tab navigation proceeds.
      dismiss(true);
    } else if (menu.contains(event.target) && ['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) {
      event.preventDefault();
      event.stopPropagation();
      const items = [...menu.querySelectorAll('button')];
      const index = items.indexOf(doc.activeElement);
      const next = event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1 : (index + (event.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length;
      items[next].focus();
    }
  };
  const reposition = () => dismiss();
  doc.addEventListener('pointerdown', outside, true);
  doc.addEventListener('keydown', keyboard, true);
  win.addEventListener('resize', reposition);
  sidebar.addEventListener('scroll', reposition, true);
  return () => {
    dismiss();
    doc.removeEventListener('pointerdown', outside, true);
    doc.removeEventListener('keydown', keyboard, true);
    win.removeEventListener('resize', reposition);
    sidebar.removeEventListener('scroll', reposition, true);
    sidebar.remove();
    style.remove();
  };
}

export function mountContext(root, state, api) {
  return mount(root, state, api, false);
}

export function mountHover(root, state, api) {
  return mount(root, state, api, true);
}
