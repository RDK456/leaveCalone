const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const $ = (s, r = document) => r.querySelector(s);
const view = $('#view');
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const GB = 1024 ** 3, MB = 1024 ** 2;
const fmt = (b) => b >= GB ? `${(b / GB).toFixed(1)}<span class="u">GB</span>` : b >= MB ? `${Math.round(b / MB).toLocaleString()}<span class="u">MB</span>` : b > 0 ? `${Math.max(1, Math.round(b / 1024)).toLocaleString()}<span class="u">KB</span>` : '<span class="mut">0</span>';
const txt = (b) => b >= GB ? `${(b / GB).toFixed(1)} GB` : `${Math.round(b / MB).toLocaleString()} MB`;
const dash = '<span class="mut">–</span>';

const S = { drives: [], cats: null, cands: null, sel: new Set(), moveDrive: null, scanning: false, last: null, exp: null, expItems: null, moved: null, links: null, cleanDrive: 'C:', dcats: null, settings: null, info: null, lastScan: null, upd: null };

function toast(msg, bad) {
  const t = document.createElement('div');
  t.className = 't' + (bad ? ' bad' : '');
  t.textContent = msg;
  $('#toast').append(t);
  setTimeout(() => t.remove(), bad ? 9000 : 5000);
}
const cDrive = () => S.drives.find((d) => d.letter.toUpperCase() === 'C:');
const otherDrives = () => S.drives.filter((d) => d.letter.toUpperCase() !== 'C:' && !d.removable);

async function refreshDrives() { S.drives = await invoke('drives'); }
async function scan() {
  S.scanning = true; render();
  try {
    await refreshDrives();
    const [cats, cands] = await Promise.all([invoke('scan_clean', { drive: 'C:' }), invoke('move_candidates')]);
    S.cats = cats; S.cands = cands;
    S.sel = new Set(cats.filter((c) => c.default_on && !c.busy && c.bytes > 0).map((c) => c.id));
    S.moveDrive = S.moveDrive || otherDrives().sort((a, b) => b.free - a.free)[0]?.letter || null;
  } catch (e) { toast(String(e), true); }
  S.scanning = false; render();
  if (window.afterScan) window.afterScan();
}
const curCats = () => (S.cleanDrive === 'C:' ? S.cats : S.dcats);
const selBytes = (l = curCats()) => (l || []).filter((c) => S.sel.has(c.id)).reduce((a, c) => a + c.bytes, 0);
const defBytes = () => (S.cats || []).filter((c) => c.default_on && !c.busy).reduce((a, c) => a + c.bytes, 0);
const cleanable = () => (S.cats || []).filter((c) => !c.busy).reduce((a, c) => a + c.bytes, 0);
const movable = () => (S.cands || []).filter((c) => c.status === 'ready').reduce((a, c) => a + c.bytes, 0);

// ---------- views ----------
function head(title, sub, acts) {
  return `<div class="title"><div><h1>${esc(title)}</h1><p class="sub">${sub || ''}</p></div><div class="acts">${acts || ''}</div></div>`;
}
function skeleton() { return `<div class="card"><div class="row"><div class="skel"></div></div><div class="row"><div class="skel" style="width:40%"></div></div><div class="row"><div class="skel" style="width:50%"></div></div></div>`; }

function vOverview() {
  const c = cDrive();
  const sc = S.cats, cd = S.cands;
  const finding = !sc ? 'Scanning C: for things that are safe to remove…'
    : `<b>${txt(cleanable())}</b> on C: is safe to clean${movable() && S.moveDrive ? ` and <b>${txt(movable())}</b> can move to ${esc(S.moveDrive)}` : ''}.`;
  const act = `<button class="btn line" data-a="rescan" ${S.scanning ? 'disabled' : ''}>${S.scanning ? 'Scanning…' : 'Rescan'}</button>` +
    `<button class="btn pri" data-a="goclean" ${!sc || defBytes() === 0 ? 'disabled' : ''}>Review ${sc ? txt(defBytes()) : ''}</button>`;
  const used = c ? c.total - c.free : 0;
  const kpis = `<div class="kpis">
    <div class="kpi"><div class="l">Free on C:</div><div class="v">${c ? fmt(c.free) : dash}</div></div>
    <div class="kpi"><div class="l">Used on C:</div><div class="v">${c ? fmt(used) : dash}</div></div>
    <div class="kpi"><div class="l">Safe to clean</div><div class="v">${sc ? fmt(cleanable()) : dash}</div></div>
    <div class="kpi"><div class="l">Can move${S.moveDrive ? ' to ' + esc(S.moveDrive) : ''}</div><div class="v">${cd ? fmt(movable()) : dash}</div></div></div>`;
  const drv = S.drives.map((d) => { const u = d.total - d.free, p = Math.round((u / d.total) * 100);
    return `<div class="drive"><div><b>${esc(d.letter)}</b><div class="mut" style="font-size:12px">${esc(d.label || 'Local disk')}</div></div>
      <div class="bar ${d.letter.toUpperCase() === 'C:' ? 'main' : ''}" role="img" aria-label="${p}% used"><i style="width:${p}%"></i></div>
      <div class="r">${txt(d.free)} free <span class="mut">of ${txt(d.total)}</span></div></div>`; }).join('');
  const topClean = (sc || []).filter((x) => x.bytes > 0).sort((a, b) => b.bytes - a.bytes).slice(0, 5);
  const topMove = (cd || []).slice(0, 5);
  const rowsC = topClean.map((x) => `<div class="row sm2"><div class="n"><b>${esc(x.name)}</b><small>${esc(x.busy ? x.busy + ' is running' : 'Safe')}</small></div><div class="r">${fmt(x.bytes)}</div><div></div></div>`).join('') || `<div class="empty">${sc ? 'Nothing to clean.' : ''}</div>`;
  const rowsM = topMove.map((x) => `<div class="row sm2"><div class="n"><b>${esc(x.name)}</b><small>${esc(x.area)}</small></div><div class="r">${fmt(x.bytes)}</div><div class="r"><button class="btn sm line" data-a="move" data-p="${esc(x.path)}" ${x.status === 'busy' ? 'disabled' : ''}>Move</button></div></div>`).join('') || `<div class="empty">${cd ? 'Nothing large to move.' : ''}</div>`;
  return `<div class="wrap">${head('Overview', finding, act)}${kpis}
    <section><h2>Drives</h2><div class="card">${drv || skeleton()}</div></section>
    ${growthHtml()}
    <section class="two"><div><h2>Largest cleanable <span><button class="link" data-v="clean">View all</button></span></h2><div class="card">${sc ? rowsC : skeleton()}</div></div>
    <div><h2>Largest movable <span><button class="link" data-v="move">View all</button></span></h2><div class="card">${cd ? rowsM : skeleton()}</div></div></section></div>`;
}

function vClean() {
  const sc = curCats();
  const isC = S.cleanDrive === 'C:';
  const rows = (sc || []).slice().sort((a, b) => b.bytes - a.bytes).map((c) => {
    const off = c.busy || c.bytes === 0;
    const pill = c.busy ? `<span class="pill warn">Close ${esc(c.busy)}</span>` : c.id === 'recycle' ? '<span class="pill no">Permanent</span>' : '<span class="pill ok">Safe</span>';
    return `<div class="row cl"><input type="checkbox" data-c="${c.id}" aria-label="${esc(c.name)}" ${S.sel.has(c.id) ? 'checked' : ''} ${off ? 'disabled' : ''}>
      <div class="n"><b>${esc(c.name)}</b><small>${esc(c.desc)}</small></div><div>${pill}</div><div class="r">${fmt(c.bytes)}</div></div>`;
  }).join('');
  const last = S.last ? `Freed <b>${txt(S.last.freed)}</b>${S.last.failed ? ` · ${S.last.failed} files were in use and kept` : ''}.` : 'Only fixed, known locations are touched. Files in use are skipped.';
  return `<div class="wrap">${head('Clean', last, `<select id="cdrv" aria-label="Drive to clean">${S.drives.map((d) => `<option value="${esc(d.letter)}" ${d.letter === S.cleanDrive ? 'selected' : ''}>${esc(d.letter)} · ${txt(d.free)} free</option>`).join('')}</select><button class="btn line" data-a="rescan" ${S.scanning ? 'disabled' : ''}>Rescan</button>`)}
    <div class="card"><div class="row cl h"><span></span><span>Item</span><span>Status</span><span class="r">Size</span></div>${sc ? rows : skeleton()}</div>
    ${isC ? `<section><h2>System tasks <span>Run with this app's administrator rights</span></h2><div class="card">
      <div class="row sm2" style="grid-template-columns:1fr 130px"><div class="n"><b>Component store cleanup</b><small>DISM StartComponentCleanup. Removes superseded Windows files. Takes a few minutes.</small></div><div class="r"><button class="btn sm line" data-a="admin" data-t="dism">Run</button></div></div>
      <div class="row sm2" style="grid-template-columns:1fr 130px"><div class="n"><b>Windows Update caches</b><small>Download and Delivery Optimization caches. Pauses update services briefly.</small></div><div class="r"><button class="btn sm line" data-a="admin" data-t="update_cache">Run</button></div></div>
      <div class="row sm2" style="grid-template-columns:1fr 130px"><div class="n"><b>Windows temp folder</b><small>C:\\Windows\\Temp, files older than an hour.</small></div><div class="r"><button class="btn sm line" data-a="admin" data-t="wintemp">Run</button></div></div></div></section>` : ''}
    </div><div class="bottom"><div class="wrap"><span>Selected <b>${txt(selBytes())}</b></span><button class="btn pri" data-a="clean" ${selBytes() === 0 || S.scanning || (!isC && !S.dcats) ? 'disabled' : ''}>Clean selected</button></div></div>`;
}

function vMove() {
  const dr = otherDrives();
  const opts = dr.map((d) => `<option value="${esc(d.letter)}" ${d.letter === S.moveDrive ? 'selected' : ''}>${esc(d.letter)} · ${txt(d.free)} free</option>`).join('');
  const rows = (S.cands || []).map((x) => {
    const pill = x.status === 'ready' ? '<span class="pill ok">Ready</span>' : x.status === 'busy' ? '<span class="pill warn">Running</span>' : '<span class="pill warn">Caution</span>';
    return `<div class="row mv"><div class="n"><b>${esc(x.name)}</b><small title="${esc(x.note || x.path)}">${esc(x.note || x.path)}</small></div><div class="mut">${esc(x.area)}</div><div class="r">${fmt(x.bytes)}</div><div>${pill}</div>
      <div class="r"><button class="btn sm line" data-a="move" data-p="${esc(x.path)}" ${x.status === 'busy' || !S.moveDrive ? 'disabled' : ''}>Move…</button></div></div>`;
  }).join('') || `<div class="empty">${S.cands ? 'No folder over 100 MB qualifies.' : ''}</div>`;
  return `<div class="wrap">${head('Move to another drive', `Copies, verifies every file, then leaves a link so apps keep working. Anything unexpected rolls back.`,
    dr.length ? `<select id="drv" aria-label="Destination drive">${opts}</select>` : '<span class="mut">No second drive found</span>')}
    <div class="card"><div class="row mv h"><span>Folder</span><span>Location</span><span class="r">Size</span><span>Status</span><span></span></div>${S.cands ? rows : skeleton()}</div>
    <p class="note">Folders already containing links, system folders and anything under 100 MB are not listed. Use Explore to pick other folders.</p></div>`;
}

async function loadExplore(path) {
  S.exp = path; S.expItems = null; render();
  try {
    if (!path) S.expItems = null;
    else if (S.fast && path.toUpperCase().startsWith(S.fast)) {
      try { S.expItems = await invoke('fast_dir', { path }); } catch (e) { toast(String(e), true); S.fast = null; S.expItems = await invoke('list_dir', { path }); }
    } else S.expItems = await invoke('list_dir', { path });
  } catch (e) { toast(String(e), true); S.expItems = []; }
  render();
}
function vExplore() {
  const p = S.exp;
  if (!p) {
    const rows = S.drives.map((d) => `<div class="row sm2" style="grid-template-columns:1fr 120px 90px 120px"><div class="n"><button class="link" data-open="${esc(d.letter + '\\')}">${esc(d.letter)} ${esc(d.label || 'Local disk')}</button></div><div class="r">${txt(d.total - d.free)} used</div><div class="r">${txt(d.free)} free</div><div class="r"><button class="btn sm line" data-a="fastscan" data-p="${esc(d.letter)}" title="Reads the NTFS file table. Seconds instead of minutes.">Fast scan</button></div></div>`).join('');
    return `<div class="wrap">${head('Explore', 'Pick a drive and drill into the folders using the most space. Fast scan reads the NTFS file table and is the quickest way to see a whole drive. It also flags large files Windows cannot find.')}${hiddenHtml()}<div class="card">${rows}</div></div>`;
  }
  const parent = p.replace(/\\+$/, '').split('\\').slice(0, -1).join('\\');
  const up = p.replace(/\\+$/, '').match(/^[A-Za-z]:$/) ? '' : (parent.match(/^[A-Za-z]:$/) ? parent + '\\' : parent);
  const items = S.expItems;
  const max = items?.[0]?.bytes || 1;
  const rows = (items || []).slice(0, 200).map((e, i) => {
    const inProf = /^[A-Za-z]:\\Users\\/i.test(e.path);
    return `<div class="row ex"><div class="n">${e.is_dir ? `<button class="link" data-open="${esc(e.path)}">${esc(e.name)}</button>` : `<b>${esc(e.name)}</b>`}${e.link ? '<small>Link</small>' : ''}</div>
      <div><div class="thin"><i class="${i === 0 ? 'top' : ''}" style="width:${Math.max(1, (e.bytes / max) * 100)}%"></i></div></div>
      <div class="r">${fmt(e.bytes)}</div><div class="r mut">${e.is_dir ? e.files.toLocaleString() + ' files' : ''}</div>
      <div class="r">${e.is_dir && inProf && !e.link && S.moveDrive ? `<button class="btn sm line" data-a="move" data-p="${esc(e.path)}">Move…</button>` : ''}</div></div>`;
  }).join('') || `<div class="empty">${items ? 'Empty or no access.' : ''}</div>`;
  return `<div class="wrap">${head('Explore', `<code>${esc(p)}</code>`, `<button class="btn line" data-open="${esc(up)}">${up ? 'Up' : 'Drives'}</button><button class="btn line" data-a="reveal" data-p="${esc(p)}">Open in Explorer</button><button class="btn line" data-a="expmode">${S.expMode === 'map' ? 'List' : 'Treemap'}</button>`)}
    ${hiddenHtml()}${S.expMode === 'map' && items ? treemapHtml(items) : ''}<div class="card" ${S.expMode === 'map' && items ? 'hidden' : ''}><div class="row ex h"><span>Name</span><span>Share</span><span class="r">Size</span><span class="r">Files</span><span></span></div>${items ? rows : skeleton()}</div></div>`;
}

function vMoved() {
  const rows = (S.moved || []).map((m) => {
    const pill = m.status === 'ok' ? '<span class="pill ok">Linked</span>' : '<span class="pill no">' + (m.status === 'broken' ? 'Target missing' : 'Not a link') + '</span>';
    return `<div class="row hs"><div class="n"><b>${esc(m.rec.src)}</b></div><div class="n"><b>${esc(m.rec.dest)}</b></div><div class="r">${fmt(m.rec.bytes)}</div><div>${pill}</div>
      <div class="r"><button class="btn sm line" data-a="back" data-p="${esc(m.rec.src)}" ${m.status !== 'ok' ? 'disabled' : ''}>Move back…</button></div></div>`;
  }).join('') || `<div class="empty">${S.moved ? 'Nothing moved by Lighten yet.' : ''}</div>`;
  const found = (S.links || []).filter((l) => !l.tracked);
  const frows = found.map((l) => `<div class="row hs"><div class="n"><b>${esc(l.src)}</b></div><div class="n"><b>${esc(l.dest)}</b></div><div class="r">${fmt(l.bytes)}</div><div><span class="pill">Not tracked</span></div><div class="r"><button class="btn sm line" data-a="adopt" data-p="${esc(l.src)}">Track</button></div></div>`).join('');
  const fsec = S.links === null ? '' : found.length ? `<section><h2>Existing links found <span>Made before Lighten. Track them to enable Move back.</span></h2><div class="card">${frows}</div></section>` : '';
  return `<div class="wrap">${head('Moved folders', 'Everything Lighten linked to another drive. Move back restores the original folder.')}
    <div class="card"><div class="row hs h"><span>Original path</span><span>Now stored at</span><span class="r">Size</span><span>Status</span><span></span></div>${S.moved ? rows : skeleton()}</div>${fsec}</div>`;
}

function readSettings() { return { auto_scan: $('#s_auto').checked, every_days: +$('#s_days').value || 7, auto_clean: $('#s_clean').checked, update_url: $('#s_url').value, tray_on_close: !!$('#s_tray')?.checked }; }
function vSettings() {
  const s = S.settings, i = S.info, u = S.upd;
  if (!s) return `<div class="wrap">${head('Settings')}${skeleton()}</div>`;
  const ls = S.lastScan;
  const lastTxt = ls ? `Last scheduled scan ${new Date(ls.when * 1000).toLocaleString()}: <b>${txt(ls.cleanable)}</b> cleanable${ls.auto_clean ? `, <b>${txt(ls.freed)}</b> cleaned` : ''}.` : 'No scheduled scan has run yet.';
  const updMsg = u ? (u.error ? `<span class="pill no">${esc(u.error)}</span>` : u.available ? `<span class="pill ok">${esc(u.latest)} available${u.delta ? ' (small delta download)' : ' (full download)'}</span>` : '<span class="pill">Up to date</span>') : '';
  const inp = 'height:36px;border:1px solid var(--control);border-radius:6px;padding:0 10px;font:inherit;background:var(--surface)';
  return `<div class="wrap">${head('Settings', `Lighten ${esc(i?.version || '')}`)}
    ${generalHtml()}
    <section><h2>Scheduled scan <span>${lastTxt}</span></h2><div class="card">
      <div class="row" style="grid-template-columns:1fr auto"><div class="n"><b>Scan in the background</b><small style="white-space:normal">Creates a Windows Task Scheduler job that runs Lighten with administrator rights and no window, even when the app is closed. Each run measures what is cleanable, saves a free-space reading for the Overview chart, and shows a notification when a lot can be cleaned or free space drops sharply. Turn it off to remove the job.</small></div><input type="checkbox" id="s_auto" aria-label="Scan in the background" ${s.auto_scan ? 'checked' : ''}></div>
      <div class="row" style="grid-template-columns:1fr auto"><div class="n"><b>Repeat every</b><small style="white-space:normal">Days between scans, 1 to 30. More frequent scans give the growth chart and the sudden-drop alert finer detail; each scan takes under a minute.</small></div><input id="s_days" aria-label="Repeat every, in days" type="number" min="1" max="30" value="${s.every_days || 7}" style="${inp};width:70px"></div>
      <div class="row" style="grid-template-columns:1fr auto"><div class="n"><b>Also clean the safe set</b><small style="white-space:normal">After each scan, delete the items that are ticked by default on the Clean page (temp files, shader caches, logs and similar that apps rebuild). It never empties the Recycle Bin, never touches browser data or the opt-in items, and skips anything in use or on your exclusion list. Off means scan and notify only.</small></div><input type="checkbox" id="s_clean" aria-label="Also clean the safe set" ${s.auto_clean ? 'checked' : ''}></div></div>
      <p class="note"><button class="btn pri" data-a="savesettings">Save schedule</button> Applies the three settings above. Nothing changes until you press Save.</p></section>
    <section><h2>Updates <span>${updMsg || (S.inst?.signing ? 'Signed updates required' : 'Signing key not built in: hash check only')}</span></h2><div class="card">
      <div class="row" style="grid-template-columns:1fr"><div class="n"><b>Update manifest URL</b><small style="white-space:normal">An https link to a small JSON file that names the newest version. Lighten only accepts it if it is signed with the key built into this copy, and checks the download against its SHA-256. When a patch exists for your version only the difference is downloaded. Leave empty to turn updates off; nothing is ever checked in the background.</small></div></div>
      <div class="row" style="grid-template-columns:1fr"><input id="s_url" aria-label="Update manifest URL" value="${esc(s.update_url)}" placeholder="https://example.com/lighten/manifest.json" style="${inp};min-width:0;width:100%"></div></div>
      <p class="note"><button class="btn line" data-a="checkupd">Check for updates</button> ${u && u.available ? '<button class="btn pri" data-a="applyupd">Install update</button>' : ''}</p></section></div>`;
}

const views = { overview: vOverview, clean: vClean, move: vMove, explore: vExplore, moved: vMoved, settings: vSettings };
let cur = 'overview';
// innerHTML replaces the focused control, so find its twin again or keyboard users land on <body>.
function focusKey(el) {
  if (!el || !view.contains(el)) return null;
  if (el.id) return `#${CSS.escape(el.id)}`;
  const k = [...el.attributes].filter((a) => a.name.startsWith('data-')).map((a) => `[${a.name}="${CSS.escape(a.value)}"]`).join('');
  return k ? el.tagName.toLowerCase() + k : null;
}
function render() {
  const y = view.scrollTop, k = focusKey(document.activeElement);
  view.innerHTML = views[cur](); view.scrollTop = y;
  if (k) view.querySelector(k)?.focus({ preventScroll: true });
}
async function go(v) {
  cur = v;
  document.querySelectorAll('.nav[data-view]').forEach((n) => { const on = n.dataset.view === v; n.classList.toggle('on', on); on ? n.setAttribute('aria-current', 'page') : n.removeAttribute('aria-current'); });
  if (v === 'explore') { await refreshDrives(); S.exp = null; }
  if (v === 'moved') { S.moved = null; S.links = null; render(); S.moved = await invoke('moved_list'); render(); S.links = await invoke('find_links'); }
  if (v === 'settings') { S.settings = null; render(); [S.settings, S.info, S.lastScan] = await Promise.all([invoke('get_settings'), invoke('app_info'), invoke('last_scan')]); }
  if (window.onGo) await window.onGo(v);
  render();
}

// ---------- dialogs ----------
const dlg = $('#dlg');
function openDlg(html) { dlg.innerHTML = html; if (!dlg.open) dlg.showModal(); }
dlg.addEventListener('click', (e) => { if (e.target.dataset.close !== undefined && !dlg.dataset.lock) dlg.close(); });
const STEPS = { copy: 'Copy files', verify: 'Verify every file', swap: 'Set original aside', link: 'Create link', clean: 'Remove old copy', unlink: 'Remove link' };
function stepsHtml(keys) { return `<ol class="steps">${keys.map((k) => `<li data-k="${k}">${STEPS[k]}</li>`).join('')}</ol>`; }
listen('move-step', (e) => {
  const li = dlg.querySelector(`li[data-k="${e.payload.key}"]`); if (!li) return;
  let seen = true;
  dlg.querySelectorAll('li').forEach((x) => { if (x === li) { x.className = 'cur'; seen = false; } else if (seen) x.className = 'done'; });
});

async function doMove(path) {
  const dr = otherDrives();
  if (!dr.length) return toast('No second drive found', true);
  if (!dr.some((d) => d.letter === S.moveDrive)) S.moveDrive = dr[0].letter;
  const show = async () => {
    let p;
    try { p = await invoke('plan_move', { src: path, drive: S.moveDrive }); } catch (e) { return toast(String(e), true); }
    const bad = p.problems.length;
    const sel = `<select id="mdrv" aria-label="Destination drive">${dr.map((d) => `<option value="${esc(d.letter)}" ${d.letter === S.moveDrive ? 'selected' : ''}>${esc(d.letter)} · ${txt(d.free)} free</option>`).join('')}</select>`;
    openDlg(`<h3>Move this folder?</h3><p class="mut">From</p><div class="path">${esc(p.src)}</div><p class="mut">To drive ${sel}, a link stays at the original path</p><div class="path">${esc(p.dest)}</div>
      <p><b>${txt(p.bytes)}</b> · ${p.files.toLocaleString()} files · ${txt(p.dest_free)} free on destination</p>
      ${bad ? `<div class="problems">${p.problems.map(esc).join('<br>')}</div>` : `<ul><li>Close the app first if it uses this folder.</li><li>Every file is checked before the original is removed.</li><li>You can move it back from Moved folders.</li></ul>`}
      <div class="foot"><button class="btn line" data-close>Cancel</button><button class="btn pri" id="go" ${bad ? 'disabled' : ''}>Move ${txt(p.bytes)}</button></div>`);
    $('#mdrv').onchange = (ev) => { S.moveDrive = ev.target.value; show(); };
    $('#go').onclick = async () => {
      dlg.dataset.lock = '1';
      openDlg(`<h3>Moving…</h3><p class="mut">${esc(p.src)}</p>${stepsHtml(['copy', 'verify', 'swap', 'link', 'clean'])}<p class="note">Keep this window open.</p>`);
      try {
        await invoke('move_folder', { src: p.src, drive: S.moveDrive });
        delete dlg.dataset.lock; dlg.close(); toast(`Moved ${txt(p.bytes)} to ${S.moveDrive}`);
        S.cands = (S.cands || []).filter((x) => x.path.toLowerCase() !== p.src.toLowerCase());
        await refreshDrives();
      } catch (e) { delete dlg.dataset.lock; openDlg(`<h3>Not moved</h3><div class="problems">${esc(e)}</div><div class="foot"><button class="btn pri" data-close>Close</button></div>`); }
      render();
    };
  };
  show();
}
async function doBack(path) {
  openDlg(`<h3>Move back?</h3><div class="path">${esc(path)}</div><p class="mut">Removes the link, copies everything back to this drive and checks it. Close the app first.</p>
    <div class="foot"><button class="btn line" data-close>Cancel</button><button class="btn pri" id="go">Move back</button></div>`);
  $('#go').onclick = async () => {
    dlg.dataset.lock = '1';
    openDlg(`<h3>Moving back…</h3>${stepsHtml(['unlink', 'copy', 'verify', 'clean'])}`);
    try { await invoke('move_back', { src: path }); delete dlg.dataset.lock; dlg.close(); toast('Moved back'); S.moved = await invoke('moved_list'); await refreshDrives(); }
    catch (e) { delete dlg.dataset.lock; openDlg(`<h3>Not moved back</h3><div class="problems">${esc(e)}</div><div class="foot"><button class="btn pri" data-close>Close</button></div>`); }
    render();
  };
}
async function loadDCats() {
  S.dcats = null; render();
  try { S.dcats = await invoke('scan_clean', { drive: S.cleanDrive }); S.sel = new Set(S.dcats.filter((c) => c.default_on && !c.busy && c.bytes > 0).map((c) => c.id)); } catch (e) { toast(String(e), true); S.dcats = []; }
  render();
}
function setCleanDrive(d) {
  S.cleanDrive = d;
  if (d === 'C:') { S.sel = new Set((S.cats || []).filter((c) => c.default_on && !c.busy && c.bytes > 0).map((c) => c.id)); render(); } else loadDCats();
}
async function doClean() {
  const list = curCats() || [], drive = S.cleanDrive, isC = drive === 'C:';
  const ids = [...S.sel].filter((id) => list.some((c) => c.id === id));
  const names = list.filter((c) => ids.includes(c.id)).map((c) => c.name);
  let rp = false; try { rp = localStorage.getItem('rp') === '1'; } catch {}
  openDlg(`<h3>Clean ${txt(selBytes())} on ${esc(drive)}?</h3><ul>${names.map((n) => `<li>${esc(n)}</li>`).join('')}</ul>
    ${ids.includes('recycle') ? '<div class="problems">Recycle Bin is emptied permanently.</div>' : ''}
    ${isC ? `<label style="display:block;margin:12px 0"><input type="checkbox" id="rp" ${rp ? 'checked' : ''}> Create a restore point first (Windows allows one per day)</label>` : ''}
    <div class="foot"><button class="btn line" data-close>Cancel</button><button class="btn line" id="dry">Export report only</button><button class="btn pri" id="go">Clean</button></div>`);
  $('#dry').onclick = async () => { try { const p = await invoke('export_report'); dlg.close(); toast('Saved ' + p); invoke('reveal_file', { path: p }); } catch (e) { toast(String(e), true); } };
  $('#go').onclick = async () => {
    $('#go').disabled = true; $('#go').textContent = 'Cleaning…';
    try {
      if (isC && $('#rp')?.checked) { try { localStorage.setItem('rp', '1'); } catch {} $('#go').textContent = 'Creating restore point…'; toast(await invoke('system_task', { task: 'restore_point', arg: null })); $('#go').textContent = 'Cleaning…'; }
      else if (isC) { try { localStorage.setItem('rp', '0'); } catch {} }
      const res = await invoke('clean', { ids, drive });
      S.last = { freed: res.reduce((a, r) => a + r.freed, 0), failed: res.reduce((a, r) => a + r.failed, 0) };
      dlg.close(); toast(`Freed ${txt(S.last.freed)}`);
      res.filter((r) => r.note).forEach((r) => toast(r.note, true));
      await refreshDrives();
      if (isC) await scan(); else await loadDCats();
    } catch (e) { dlg.close(); toast(String(e), true); }
  };
}

// ---------- events ----------
document.addEventListener('click', async (e) => {
  const nav = e.target.closest('[data-view]'); if (nav) return go(nav.dataset.view);
  const v = e.target.closest('[data-v]'); if (v) return go(v.dataset.v);
  const o = e.target.closest('[data-open]'); if (o) return loadExplore(o.dataset.open || null);
  const a = e.target.closest('[data-a]'); if (!a) return;
  const act = a.dataset.a;
  if (act === 'rescan') return S.cleanDrive === 'C:' ? scan() : loadDCats();
  if (act === 'goclean') { S.cleanDrive = 'C:'; S.sel = new Set((S.cats || []).filter((c) => c.default_on && !c.busy && c.bytes > 0).map((c) => c.id)); return go('clean'); }
  if (act === 'clean') return doClean();
  if (act === 'move') return doMove(a.dataset.p);
  if (act === 'back') return doBack(a.dataset.p);
  if (act === 'adopt') { try { await invoke('adopt_link', { src: a.dataset.p }); S.moved = await invoke('moved_list'); S.links = await invoke('find_links'); toast('Tracked'); } catch (er) { toast(String(er), true); } return render(); }
  if (act === 'savesettings') {
    try { S.settings = await invoke('save_settings', { s: readSettings() }); toast(S.settings.auto_scan ? `Scan scheduled every ${S.settings.every_days} day(s)` : 'Schedule off'); } catch (er) { toast(String(er), true); }
    return render();
  }
  if (act === 'checkupd') {
    try { S.settings = await invoke('save_settings', { s: readSettings() }); S.upd = await invoke('check_update'); } catch (er) { S.upd = { error: String(er) }; }
    return render();
  }
  if (act === 'applyupd') { try { await invoke('apply_update'); } catch (er) { toast(String(er), true); } return; }
  if (act === 'reveal') return invoke('open_in_explorer', { path: a.dataset.p });
  if (act === 'admin') {
    a.disabled = true; const before = cDrive()?.free || 0; toast('Running… this can take a few minutes');
    try { await invoke('admin_task', { task: a.dataset.t }); await refreshDrives(); const d = (cDrive()?.free || 0) - before; toast(d > 0 ? `Done. ${txt(d)} freed` : 'Done. Little or nothing to free'); await scan(); }
    catch (er) { toast(String(er), true); }
    a.disabled = false;
  }
});
document.addEventListener('change', (e) => {
  if (e.target.dataset.c) { e.target.checked ? S.sel.add(e.target.dataset.c) : S.sel.delete(e.target.dataset.c); render(); }
  if (e.target.id === 'drv') { S.moveDrive = e.target.value; render(); }
});
window.addEventListener('load', scan);
setInterval(() => { if (!dlg.open && !S.scanning && document.visibilityState === 'visible' && cur !== 'explore') scan(); }, 30 * 60 * 1000);
