// THROWAWAY browser adapter. Hierarchy/labels/actions come from the production declaration.
import { declareTree } from './production-sidebar.prototype.js';
const CSS = `
.production-sidebar{font:16px/22px "JetBrains Mono","Fira Code",ui-monospace,monospace;background:#0d0a10;color:#b9b3bd;min-height:100%;height:100%;display:flex;flex-direction:column;padding:0}
.production-sidebar .production-list{overflow:auto;min-height:0}
.production-sidebar .production-row{height:22px;min-height:22px;position:relative;display:flex;align-items:center;border-radius:0;gap:0;padding:0 1ch 0 3ch;background:transparent}
.production-sidebar .production-row.section{padding-left:1ch;gap:1ch}
.production-sidebar .production-row.blank{padding:0}
.production-sidebar .production-select{font:inherit;color:inherit;display:flex;align-items:center;gap:0;min-width:0;flex:1;height:100%;padding:0;border:0;background:transparent;text-align:left;cursor:pointer;white-space:pre}
.production-sidebar .production-title{width:100%;text-align:center;color:#6f6875}
.production-sidebar .production-row.selected{background:#1a263d;color:#f2eef5;box-shadow:inset 2px 0 #4d7fb8}
.production-sidebar .production-row.selected [data-role=label]{font-weight:700}
.production-sidebar .production-select:focus-visible{outline:1px solid #4d7fb8;outline-offset:-1px}
.production-sidebar [data-role=section]{font-weight:700;overflow:hidden;text-overflow:ellipsis}
.production-sidebar [data-role=muted],.production-sidebar [data-role=title]{color:#6f6875}
.production-sidebar [data-role=status-working]{color:#e2a23a}
.production-sidebar [data-role=status-blocked]{color:#d05a55}
.production-sidebar [data-role=status-waiting]{color:#6f8fc0}
.production-sidebar [data-role=status-idle]{color:#4a4550}
.production-sidebar [data-role=status-done]{color:#7bb879}
.production-sidebar .production-leader{flex:1;min-width:1ch;height:1px;background:#1e1b21;margin:0 1ch}
.production-sidebar .production-row.section .production-select{gap:0}
.production-sidebar .production-row.section [data-role^=status-]{flex:none}
.production-sidebar .chp-trigger{position:absolute;right:0;top:0;z-index:1;width:2ch;height:22px;min-height:22px;border-radius:0;background:#0d0a10;color:#b9b3bd;font:inherit;border:0;cursor:pointer}
.production-sidebar .production-row.selected .chp-trigger{background:#1a263d}
.production-sidebar .production-row:hover>.chp-trigger,.production-sidebar .production-row:focus-within>.chp-trigger{visibility:visible;opacity:1}
.production-sidebar .production-row:has(>.chp-trigger):is(:hover,:focus-within)>.production-select{padding-right:2ch}
.production-sidebar .production-footer{margin-top:auto;height:22px;position:relative;padding:0 1ch;font:inherit}
.production-sidebar .production-footer .chp-trigger{visibility:hidden;opacity:0}
.production-sidebar .production-footer:hover>.chp-trigger,.production-sidebar .production-footer:focus-within>.chp-trigger{visibility:visible;opacity:1}
`;
export function mountProductionRows(sidebar,state,api,onRow) {
  const doc=sidebar.ownerDocument;
  const style=doc.createElement('style');style.textContent=CSS;sidebar.append(style);
  sidebar.classList.add('production-sidebar');
  state.collapsedSpaces??=[];
  const entities=[];
  state.spaces.forEach((space,w)=>{
    entities.push({kind:'workspace',number:w,position:w,workspaceId:space.id,label:space.name});
    space.tabs.forEach((tab,t)=>{
      entities.push({kind:'tab',number:t,position:t,workspaceId:space.id,tabId:tab.id,label:tab.name,agentStatus:tab.status??tab.panes.find(p=>p.status!=='idle')?.status??'idle'});
      tab.panes.forEach((pane,p)=>entities.push({kind:'pane',number:p,position:p,workspaceId:space.id,tabId:tab.id,paneId:pane.id,label:pane.name,agentStatus:pane.status}));
    });
  });
  const rows=declareTree('junkman',entities,new Set(state.collapsedSpaces)).zones[0].tree.children;
  // Visible state/evidence uses the very same declaration consumed by this renderer.
  state.sidebarDeclaration=rows;
  const list=doc.createElement('div');list.className='production-list';sidebar.append(list);
  for(const row of rows){
    const context=row.context??{};
    const space=state.spaces.find(s=>s.id===context.workspace_id);
    const tab=space?.tabs.find(t=>t.id===context.tab_id);
    const pane=tab?.panes.find(p=>p.id===context.pane_id);
    const section=row.action==='open-workspace';
    const line=doc.createElement('div');line.className='production-row'+(section?' section':'')+(!row.action?' blank':'');
    if(row.action==='focus-tab'&&state.selectedTab===tab.id)line.classList.add('selected');
    const button=doc.createElement(row.action?'button':'div');button.className='production-select';
    if(row.action){
      button.type='button';button.dataset.spaceId=space.id;
      button.setAttribute('aria-label',`${section?'space':pane?'pane':'tab'}: ${section?space.name:pane?pane.name:tab.name}`);
      if(section)button.setAttribute('aria-expanded',String(!state.collapsedSpaces.includes(space.id)));
      if(line.classList.contains('selected'))button.setAttribute('aria-current','true');
      button.addEventListener('click',()=>{
        if(section){const i=state.collapsedSpaces.indexOf(space.id);if(i<0)state.collapsedSpaces.push(space.id);else state.collapsedSpaces.splice(i,1);api.refresh()}
        else api.select(space.id,tab.id,pane?.id);
      });
    }
    for(const child of row.children){
      const part=doc.createElement('span');
      if(child.type==='leader')part.className='production-leader';
      else {part.dataset.role=child.role;part.textContent=child.text;if(child.role==='title')part.className='production-title'}
      button.append(part);
    }
    line.append(button);list.append(line);
    if(row.action)onRow?.({line,button,space,tab,pane,section});
  }
  return list;
}
