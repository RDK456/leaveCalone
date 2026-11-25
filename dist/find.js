// Find files: nine finders in one view. Loaded before tools.js; shares globals with app.js.
const FT = { large: 'Large files', dupes: 'Duplicates', photos: 'Similar photos', installers: 'Installers', dev: 'Dev leftovers', empty: 'Empty folders', shortcuts: 'Broken shortcuts', onedrive: 'OneDrive', undo: 'Undo' };
const FCFG = { large: ['root', 'min'], dupes: ['root', 'min'], photos: ['root', 'dist', 'cap'], installers: ['days'], dev: ['root', 'days'], empty: ['root'], shortcuts: [], onedrive: [], undo: [] };
const FAUTO = ['shortcuts', 'onedrive', 'undo']; // nothing to choose: they run when opened
const FNOTE = {
  large: 'The biggest files in a folder. On NTFS drives the file table is read directly, so a whole drive takes seconds. Windows and Program Files are skipped.',
  dupes: 'Identical files, matched by content hash. The oldest copy in each set is kept unless you change it.',
  photos: 'Photos that look alike even when resized or re-saved, matched by a visual fingerprint. The largest in each set is kept. Check each set before you delete. Only the first photos found are checked; raise the limit for big libraries (slower).',
  installers: 'iso, msi, msix and exe files in Downloads, plus iso/msi/msix on the Desktop and in Documents.',
  dev: 'node_modules, Rust target and Python venv folders in projects you have not touched. They are rebuilt by npm, cargo or pip.',
  empty: 'Folders with nothing inside, not even empty subfolders. Nothing is selected for you: some programs expect an empty folder to exist.',
  shortcuts: 'Desktop and Start menu shortcuts whose target no longer exists. Shortcuts to network drives, unplugged drives and Store apps are left alone.',
  onedrive: 'Files that are on this PC and in your cloud. Freeing space keeps them in OneDrive and downloads them again when opened. Files not yet uploaded are refused.',
  undo: 'Files Lighten sent to the Recycle Bin that are still in it (emptied ones are not listed). Restore puts them back where they were.',
};
S.find = { q: '', run: 0, tab: 'large', root: 'C:\\Users', min: 100, days: 90, dist: 4, cap: 6000, res: null, busy: false, sel: new Set(), size: new Map(), perm: false, roots: [] };
S.profile = null;
const isGrp = () => S.find.tab === 'dupes' || S.find.tab === 'photos';

function findTab(k) {
  const f = S.find;
  const other = S.drives.find((d) => d.letter !== 'C:');
  const prof = S.profile || 'C:\\Users';
  if (k === 'dev' && (f.root === 'C:\\Users' || f.root === prof || f.root.toLowerCase().endsWith('\\pictures'))) f.root = other ? other.letter + '\\' : prof;
  else if (k === 'photos') f.root = prof + '\\Pictures';
  else if (k === 'empty') f.root = prof;
  else if (['large', 'dupes'].includes(k) && f.root.toLowerCase().endsWith('\\pictures')) f.root = 'C:\\Users';
  if (f.busy) { invoke('cancel_scan'); unwatchScan(); } // leaving a running scan stops it
  f.run++;
  Object.assign(f, { tab: k, res: null, sel: new Set(), size: new Map(), busy: false, q: '' });
  S.sorts.find = null;
  render();
  if (FAUTO.includes(k)) findRun();
}
const matches = (h, q) => `${h.path} ${h.target || ''}`.toLowerCase().includes(q);
/** What is on screen: the search filter, then the column sort. A matching group is shown whole, so a kept copy stays visible. */
function shownRows() {
  const f = S.find, q = f.q.trim().toLowerCase(), rows = f.res || [];
  if (isGrp()) return q ? rows.filter((g) => g.files.some((h) => matches(h, q))) : rows;
  return sortRows(q ? rows.filter((h) => matches(h, q)) : rows, S.sorts.find);
}
const shownPaths = () => (isGrp() ? shownRows().flatMap((g) => g.files) : shownRows()).map((h) => h.path);
function hiddenSelected() {
  const seen = new Set(shownPaths());
  return [...S.find.sel].filter((p) => !seen.has(p)).length;
}
function csvRows() {
  const day = (s) => (s ? new Date(s * 1000).toISOString().slice(0, 10) : '');
  if (isGrp()) return [['Set', 'Name', 'Path', 'Bytes', 'Modified'], ...shownRows().flatMap((g, i) => g.files.map((h) => [String(i + 1), h.name, h.path, String(h.bytes), day(h.modified)]))];
  if (S.find.tab === 'dev') return [['Kind', 'Project', 'Path', 'Bytes', 'Idle days'], ...shownRows().map((h) => [h.kind, h.project, h.path, String(h.bytes), String(h.idle_days)])];
  return [['Name', 'Path', 'Bytes', 'Modified', 'Target'], ...shownRows().map((h) => [h.name, h.path, String(h.bytes ?? 0), day(h.modified), h.target || ''])];
}
function colHead() {
  const f = S.find;
  if (f.tab === 'dev') return `<div class="row h" style="grid-template-columns:24px 1fr 110px 100px 90px"><span></span>${sortBtn('find', 'project', 'Project')}${sortBtn('find', 'idle_days', 'Idle', true)}${sortBtn('find', 'bytes', 'Size', true)}<span></span></div>`;
  const sized = !['empty', 'shortcuts'].includes(f.tab);
  return `<div class="row h" style="grid-template-columns:24px 1fr 100px 110px 70px"><span></span>${sortBtn('find', 'name', 'Name')}${sortBtn('find', 'modified', f.tab === 'undo' ? 'Deleted' : 'Modified', true)}${sized ? sortBtn('find', 'bytes', 'Size', true) : '<span></span>'}<span></span></div>`;
}
function vFind() {
  const f = S.find, cfg = FCFG[f.tab];
  const tabs = Object.entries(FT).map(([k, v]) => `<button class="tab ${f.tab === k ? 'on' : ''}" data-ft="${k}"${f.tab === k ? ' aria-current="true"' : ''}>${v}</button>`).join('');
  const roots = driveRoots().map((r) => `<option value="${esc(r)}">`).join('');
  const bar = `<div class="bar2">${cfg.includes('root') ? `<input class="t" id="f_root" list="roots" value="${esc(f.root)}" style="width:240px" aria-label="Folder to scan"><datalist id="roots">${roots}</datalist>` : ''}
    ${cfg.includes('min') ? `<select id="f_min" aria-label="Minimum size">${[10, 100, 500, 1000].map((m) => `<option value="${m}" ${f.min === m ? 'selected' : ''}>Over ${m >= 1000 ? '1 GB' : m + ' MB'}</option>`).join('')}</select>` : ''}
    ${cfg.includes('days') ? `<select id="f_days" aria-label="Older than">${[30, 60, 90, 180, 365].map((d) => `<option value="${d}" ${f.days === d ? 'selected' : ''}>${f.tab === 'dev' ? 'Idle' : 'Older than'} ${d} days</option>`).join('')}</select>` : ''}
    ${cfg.includes('dist') ? `<select id="f_dist" aria-label="How alike"><option value="4" ${f.dist === 4 ? 'selected' : ''}>Near-identical</option><option value="8" ${f.dist === 8 ? 'selected' : ''}>Similar</option></select>` : ''}
    ${cfg.includes('cap') ? `<select id="f_cap" aria-label="How many photos to check">${[6000, 20000, 50000, 100000].map((c) => `<option value="${c}" ${f.cap === c ? 'selected' : ''}>Check up to ${c.toLocaleString()}</option>`).join('')}</select>` : ''}
    ${f.res?.length ? `<input class="t" id="f_q" type="search" placeholder="Search results" aria-label="Search results" value="${esc(f.q)}" style="width:200px">` : ''}
    <button class="btn pri" data-fa="run" ${f.busy ? 'disabled' : ''}>${f.busy ? 'Scanning…' : FAUTO.includes(f.tab) ? 'Rescan' : 'Scan'}</button>
    ${['empty', 'onedrive', 'undo'].includes(f.tab) && f.res?.length ? '<button class="btn line" data-fa="all">Select all</button>' : ''}${f.res?.length ? '<button class="btn line" data-fa="csv">Export CSV</button>' : ''}</div>`;
  let body = '';
  const rows = f.res === null ? null : shownRows();
  if (rows === null) body = `<div class="card"><div class="empty">${f.busy ? 'Scanning, this can take a minute…' + (['shortcuts', 'undo'].includes(f.tab) ? '' : progressHtml()) : 'Choose options and press Scan.'}</div></div>`;
  else if (f.res.length && !rows.length) body = '<div class="card"><div class="empty">No result matches your search.</div></div>';
  else if (!rows.length) body = `<div class="card"><div class="empty">${f.tab === 'onedrive' && !f.roots.length ? 'OneDrive was not found on this PC.' : 'Nothing found.'}</div></div>`;
  else if (isGrp()) {
    const label = f.tab === 'photos' ? (g) => `${g.files.length} similar photos · largest ${txt(g.bytes)} · ${txt(g.files.slice(1).reduce((a, h) => a + h.bytes, 0))} reclaimable` : (g) => `${g.files.length} copies · ${txt(g.bytes)} each · ${txt(g.bytes * (g.files.length - 1))} wasted`;
    body = `<div class="card">${rows.map((g) => `<div class="grp">${label(g)}</div>${g.files.map((h) => fileRow(h)).join('')}`).join('')}</div>`;
  } else if (f.tab === 'dev') {
    body = `<div class="card">${colHead()}${rows.map((h) => `<div class="row" style="grid-template-columns:24px 1fr 110px 100px 90px"><input type="checkbox" data-fp="${esc(h.path)}" aria-label="${esc(h.project)}" ${f.sel.has(h.path) ? 'checked' : ''}><div class="n"><b>${esc(h.project)}</b><small>${esc(h.kind)} · ${esc(h.path)}</small></div><div class="r mut">idle ${h.idle_days} d</div><div class="r">${fmt(h.bytes)}</div><div class="r"><button class="btn sm line" data-fa="reveal" data-p="${esc(h.path)}">Show</button></div></div>`).join('')}</div>`;
  } else body = `<div class="card">${colHead()}${rows.map((h) => fileRow(h)).join('')}</div>`;
  const sb = [...f.sel].reduce((a, p) => a + (f.size.get(p) || 0), 0);
  const act = { dev: 'Delete folders', empty: 'Remove folders', onedrive: 'Free up space', undo: 'Restore' }[f.tab] || (f.perm ? 'Delete permanently' : 'Move to Recycle Bin');
  const permOk = !['dev', 'empty', 'onedrive', 'undo'].includes(f.tab);
  const sized = !['empty', 'shortcuts', 'undo'].includes(f.tab);
  return `<div class="wrap">${head('Find files', FNOTE[f.tab])}<div class="tabs">${tabs}</div>${bar}${body}</div>
    <div class="bottom"><div class="wrap"><span>Selected <b>${f.sel.size}</b>${sized ? ` · <b>${txt(sb)}</b>` : ''}${f.q && hiddenSelected() ? ` <span class="pill warn">${hiddenSelected()} hidden by search</span>` : ''}${permOk ? ` <label style="margin-left:16px"><input type="checkbox" id="f_perm" ${f.perm ? 'checked' : ''}> Delete permanently</label>` : ''}</span><button class="btn pri" data-fa="do" ${f.sel.size ? '' : 'disabled'}>${act}</button></div></div>`;
}
function fileRow(h) {
  const f = S.find;
  const sub = h.target ? `Target missing: ${esc(h.target)}` : esc(h.path);
  const size = ['empty', 'shortcuts'].includes(f.tab) ? '' : fmt(h.bytes);
  return `<div class="row" style="grid-template-columns:24px 1fr 100px 110px 70px"><input type="checkbox" data-fp="${esc(h.path)}" aria-label="${esc(h.name)}" ${f.sel.has(h.path) ? 'checked' : ''}><div class="n"><b>${esc(h.name)}</b><small title="${esc(h.path)}">${sub}</small></div><div class="r mut">${dateOf(h.modified)}</div><div class="r">${size}</div><div class="r"><button class="btn sm line" data-fa="reveal" data-p="${esc(h.path)}">Show</button></div></div>`;
}
async function findRun() {
  const f = S.find;
  f.q = ''; S.sorts.find = null;
  f.root = $('#f_root')?.value ?? f.root; f.min = +($('#f_min')?.value ?? f.min); f.days = +($('#f_days')?.value ?? f.days); f.dist = +($('#f_dist')?.value ?? f.dist); f.cap = +($('#f_cap')?.value ?? f.cap);
  const tab = f.tab, run = ++f.run; // a later run or a tab switch makes this one stale
  f.busy = true; f.res = null; f.sel = new Set(); f.size = new Map(); render(); watchScan();
  try {
    let res;
    if (tab === 'large') res = await invoke('find_large', { root: f.root, minMb: f.min });
    else if (tab === 'dupes') res = await invoke('find_dupes', { root: f.root, minMb: f.min });
    else if (tab === 'photos') res = await invoke('find_similar', { root: f.root, maxDist: f.dist, cap: f.cap });
    else if (tab === 'installers') res = await invoke('find_installers', { days: f.days });
    else if (tab === 'dev') res = await invoke('find_dev', { root: f.root, days: f.days });
    else if (tab === 'empty') res = (await invoke('find_empty', { root: f.root })).map((h) => ({ ...h, bytes: 0 }));
    else if (tab === 'shortcuts') res = await invoke('find_shortcuts');
    else if (tab === 'onedrive') { const o = await invoke('onedrive_scan'); f.roots = o.roots; res = o.files; }
    else res = (await invoke('trash_log')).map((t) => ({ path: t.path, name: t.path.split('\\').pop(), bytes: t.bytes, modified: t.when }));
    if (f.run !== run) return;
    f.res = res;
    const flat = isGrp() ? res.flatMap((g) => g.files) : res;
    flat.forEach((h) => f.size.set(h.path, h.bytes));
    if (['installers', 'dev', 'shortcuts'].includes(tab)) flat.forEach((h) => f.sel.add(h.path));
    if (isGrp()) res.forEach((g) => g.files.slice(1).forEach((h) => f.sel.add(h.path)));
  } catch (e) {
    if (isCancel(e)) toast('Scan cancelled'); else { toast(String(e), true); if (f.run === run) f.res = []; }
  }
  if (f.run === run) { unwatchScan(); f.busy = false; render(); }
}
function findDo() {
  const f = S.find, paths = [...f.sel], sb = paths.reduce((a, p) => a + (f.size.get(p) || 0), 0), t = f.tab;
  const kind = { dev: 'dev', empty: 'empty', onedrive: 'od', undo: 'undo' }[t] || 'trash';
  const perm = kind === 'trash' && f.perm;
  const T = {
    dev: ['Delete', 'These are rebuilt by your package manager, but deleting cannot be undone.'],
    empty: ['Remove', 'Only folders that are still empty are removed.'],
    od: ['Free up space on', 'The files stay in OneDrive and download again when you open them. Files not uploaded yet are skipped.'],
    undo: ['Restore', 'Files go back to the place they were deleted from.'],
    trash: [perm ? 'Permanently delete' : 'Move to Recycle Bin', perm ? 'Permanent. This cannot be undone.' : ''],
  }[kind];
  const showSize = !['empty', 'shortcuts', 'undo'].includes(t);
  const unseen = hiddenSelected();
  openDlg(`<h3>${T[0]} ${paths.length} item(s)${showSize ? ', ' + txt(sb) : ''}?</h3>
    <ul>${paths.slice(0, 6).map((p) => `<li style="word-break:break-all">${esc(p)}</li>`).join('')}${paths.length > 6 ? `<li>and ${paths.length - 6} more</li>` : ''}</ul>
    ${unseen ? `<div class="problems">${unseen} of these are hidden by your search and are included.</div>` : ''}
    ${kind === 'dev' || perm ? `<div class="problems">${esc(T[1])}</div>` : `<p class="mut">${kind === 'trash' ? 'You can restore them from the Recycle Bin, or from Undo here. Space is freed when it is emptied.' : esc(T[1])}</p>`}
    <div class="foot"><button class="btn line" data-close>Cancel</button><button class="btn pri" id="go">${T[0].replace(' on', '')}</button></div>`);
  $('#go').onclick = async () => {
    $('#go').disabled = true; $('#go').textContent = 'Working…';
    try {
      let msg;
      if (kind === 'dev') { const r = await invoke('delete_dev', { paths }); msg = `${r.done} done${r.failed ? `, ${r.failed} skipped` : ''}, freed ${txt(r.freed)}`; }
      else if (kind === 'empty') { const r = await invoke('delete_empty', { paths }); msg = `${r.done} folder(s) removed${r.failed ? `, ${r.failed} kept` : ''}`; }
      else if (kind === 'od') { const r = await invoke('onedrive_free', { paths }); msg = `${r.done} file(s) now online-only, ${txt(r.freed)} freed${r.failed ? `, ${r.failed} refused` : ''}`; }
      else if (kind === 'undo') { const n = await invoke('restore_trashed', { paths }); msg = `${n} file(s) restored`; }
      else { const r = await invoke('trash_files', { paths, permanent: perm }); msg = `${r.done} done${r.failed ? `, ${r.failed} skipped` : ''}${perm ? `, freed ${txt(r.freed)}` : ', in the Recycle Bin'}`; }
      dlg.close(); toast(msg);
      await refreshDrives(); await findRun();
    } catch (e) { dlg.close(); toast(String(e), true); }
  };
}
