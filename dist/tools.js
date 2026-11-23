// Find, Apps, System, growth, general settings. Shares globals with app.js.
const dateOf = (sec) => (sec ? new Date(sec * 1000).toLocaleDateString() : '–');
const driveRoots = () => ['C:\\Users', ...S.drives.map((d) => d.letter + '\\')];

// ---------- growth ----------
function chartSvg(series) {
  const W = 600, H = 150, L = 46, R = 10, T = 10, B = 22;
  const xs = series.map((s) => s[0]), ys = series.map((s) => s[1] / GB);
  const x0 = Math.min(...xs), x1 = Math.max(...xs);
  let lo = Math.floor(Math.min(...ys) - 1), hi = Math.ceil(Math.max(...ys) + 1);
  if (hi - lo < 4) hi = lo + 4;
  const X = (t) => L + (W - L - R) * (x1 === x0 ? 1 : (t - x0) / (x1 - x0));
  const Y = (v) => H - B - (H - B - T) * ((v - lo) / (hi - lo));
  const ticks = [lo, lo + (hi - lo) / 2, hi];
  const grid = ticks.map((v) => `<line x1="${L}" x2="${W - R}" y1="${Y(v)}" y2="${Y(v)}" style="stroke:var(--border)" stroke-width="1"/><text x="${L - 6}" y="${Y(v) + 4}" text-anchor="end" font-size="11" style="fill:var(--muted)">${Math.round(v)}</text>`).join('');
  const pts = series.map((s) => `${X(s[0]).toFixed(1)},${Y(s[1] / GB).toFixed(1)}`).join(' ');
  const last = series[series.length - 1];
  return `<svg viewBox="0 0 ${W} ${H}" width="100%" role="img" aria-label="Free space on C: over time">${grid}
    <polyline points="${pts}" fill="none" style="stroke:var(--hue)" stroke-width="2"/><circle cx="${X(last[0])}" cy="${Y(last[1] / GB)}" r="3.5" style="fill:var(--hue)"/>
    <text x="${L}" y="${H - 5}" font-size="11" style="fill:var(--muted)">${dateOf(x0)}</text><text x="${W - R}" y="${H - 5}" text-anchor="end" font-size="11" style="fill:var(--muted)">${dateOf(x1)}</text></svg>`;
}
function dropBanner(g) {
  if (!g.drop || g.drop < 10 * 2 ** 30) return '';
  const why = g.unexplained > g.drop / 2
    ? `The folders Lighten tracks explain only ${txt(g.drop - g.unexplained)} of it. <button class="link" data-v="explore">Run Fast scan in Explore</button> to find the rest, including files Windows hides.`
    : 'The "Grew most" list shows where it went.';
  return `<div class="alert"><b>Free space on C: fell by ${txt(g.drop)}</b> since ${dateOf(g.since)}. ${why}</div>`;
}
function growthHtml() {
  const g = S.growth;
  if (!g) return '';
  const banner = dropBanner(g);
  return banner + growthBody(g);
}
function growthBody(g) {
  if (g.series.length < 2) return `<section><h2>Growth</h2><div class="card"><div class="empty">Needs two scans to show what grew. Lighten records one when it opens (every 6 hours) and on each scheduled scan.</div></div></section>`;
  const rows = g.growers.map((x) => `<div class="row sm2"><div class="n"><b>${esc(x[0])}</b></div><div class="r">+${fmt(x[1])}</div><div class="r mut">${txt(x[2])}</div></div>`).join('') || '<div class="empty">Nothing grew by more than 50 MB.</div>';
  return `<section class="two"><div><h2>Free space on C: <span>${g.series.length} scans</span></h2><div class="card" style="padding:12px 16px">${chartSvg(g.series)}</div></div>
    <div><h2>Grew most <span>last ${g.days || 1} day(s), GB</span></h2><div class="card">${rows}</div></div></section>`;
}
async function loadGrowth() { try { S.growth = await invoke('growth'); } catch {} if (cur === 'overview') render(); }
window.afterScan = () => { invoke('snapshot', { force: false }).then(loadGrowth).catch(() => {}); loadGrowth(); };

// ---------- Apps ----------
S.apps = null; S.appq = '';
function vApps() {
  const q = S.appq.toLowerCase();
  const list = (S.apps || []).filter((a) => !q || (a.name + a.publisher).toLowerCase().includes(q)).sort((a, b) => b.bytes - a.bytes);
  const rows = list.map((a) => `<div class="row" style="grid-template-columns:1fr 90px 90px 100px 170px"><div class="n"><b>${esc(a.name)}</b><small title="${esc(a.location)}">${esc(a.publisher || '–')}${a.location ? ' · ' + esc(a.location) : ''}</small></div><div class="mut">${esc(a.version)}</div><div class="r">${a.bytes ? fmt(a.bytes) : dash}</div><div class="r mut">${a.last_used ? dateOf(a.last_used) : '–'}</div>
    <div class="r">${a.movable ? `<button class="btn sm line" data-ap="move" data-p="${esc(a.location)}">Move…</button> ` : ''}<button class="btn sm line" data-ap="uninstall" data-id="${esc(a.id)}">Uninstall</button></div></div>`).join('') || `<div class="empty">${S.apps ? 'No apps match.' : ''}</div>`;
  return `<div class="wrap">${head('Installed apps', 'Sorted by size. "Last used" comes from Windows Prefetch and may be blank.', `<input class="t" id="a_q" placeholder="Search" value="${esc(S.appq)}" style="width:220px;height:36px;border:1px solid var(--control);border-radius:6px;padding:0 10px;font:inherit">`)}
    <div class="card"><div class="row h" style="grid-template-columns:1fr 90px 90px 100px 170px"><span>App</span><span>Version</span><span class="r">Size</span><span class="r">Last used</span><span></span></div>${S.apps ? rows : skeleton()}</div>
    <p class="note">Move works for apps directly inside Program Files or your profile. Apps with a running service, or Microsoft system components, are refused.</p></div>`;
}

// ---------- System ----------
S.sys = null;
function sysRow(title, sub, btns) { return `<div class="row" style="grid-template-columns:1fr auto"><div class="n"><b>${title}</b><small style="white-space:normal">${sub}</small></div><div class="r" style="display:flex;gap:8px">${btns || ''}</div></div>`; }
function vSystem() {
  const s = S.sys;
  if (!s) return `<div class="wrap">${head('System', 'Needs administrator rights, which Lighten already has.')}${skeleton()}</div>`;
  const b = (t, label, a = '') => `<button class="btn sm line" data-sa="${t}" ${a}>${label}</button>`;
  const wo = s.windows_old ? sysRow(`Windows.old <span class="u">${txt(s.windows_old)}</span>`, 'Your previous Windows. Removing it means you cannot roll back to that version. Windows deletes it itself after about 10 days.', b('winold', 'Remove…')) : sysRow('Windows.old', 'Not present.', '');
  const hib = sysRow(`Hibernation file <span class="u">${s.hiber ? txt(s.hiber) : 'off'}</span>`, 'Turning hibernation off frees this file and also disables Fast Startup.', s.hiber ? b('hiber_off', 'Turn off…') : b('hiber_on', 'Turn on'));
  const pf = sysRow('Page file', s.pagefile.length ? s.pagefile.map(esc).join('<br>') : 'System managed. Shrinking it is not recommended, so Lighten only reports it.', '');
  const dk = s.docker ? sysRow('Docker', s.docker.map(esc).join('<br>') || 'No data', b('docker_prune', 'Prune unused') + b('docker_prune_all', 'Prune all unused…')) : sysRow('Docker', 'Not installed or not running.', '');
  const vh = s.vhdx.length ? s.vhdx.map((v) => sysRow(`Virtual disk <span class="u">${txt(v.bytes)}</span>`, esc(v.path), b('wsl_compact', 'Compact…', `data-p="${esc(v.path)}"`))).join('') : sysRow('WSL and Docker disks', 'None found.', '');
  const rp = sysRow(`Restore points ${s.restore_used ? `<span class="u">${esc(s.restore_used)} used</span>` : ''}`, 'Create a Windows restore point now, or delete the old ones to free the space they hold. Windows allows one new point every 24 hours, and System Protection must be on.', b('restore_point', 'Create') + b('restore_delete', 'Delete all…'));
  const kr = sysRow(`Windows crash reports <span class="u">${txt(s.kernel_reports || 0)}</span>`, 'Memory dumps Windows keeps after hardware or driver hiccups (LiveKernelReports). Safe to clear; Windows writes new ones only if the problem happens again.', s.kernel_reports ? b('kernel_reports', 'Clear…') : '');
  const ck = sysRow('Check disk for errors', 'A read-only scan of C: for file-system problems, run while Windows is in use. Takes under a minute on most PCs and changes nothing.', b('disk_scan_check', 'Scan'));
  return `<div class="wrap">${head('System', 'Needs administrator rights, which Lighten already has.', `<button class="btn line" data-sa="refresh">Refresh</button>`)}<div class="card">${wo}${hib}${pf}${dk}${vh}${winsxsRow()}${kr}${rp}${ck}</div>${startupHtml()}</div>`;
}
const SYS_WARN = {
  winold: 'Permanently deletes C:\\Windows.old. You will not be able to go back to the previous Windows version.',
  hiber_off: 'Hibernation and Fast Startup will stop working until you turn it back on.',
  docker_prune_all: 'Removes every image not used by a container, not only dangling ones. They are downloaded again when needed.',
  restore_delete: 'Deletes every restore point on C:. You will not be able to roll Windows back to an earlier date until a new one is created.',
  kernel_reports: 'Deletes the crash reports in C:\\Windows\\LiveKernelReports. Nothing else is touched.',
  wsl_compact: 'Shuts down WSL and every running distro, then compacts the disk. Save your work first.',
};
async function sysRun(task, arg) {
  const go = async () => {
    openDlg(`<h3>Working…</h3><p class="mut">This can take a few minutes. Keep Lighten open.</p>`); dlg.dataset.lock = '1';
    try { const out = await invoke('system_task', { task, arg: arg || null }); delete dlg.dataset.lock; openDlg(`<h3>Done</h3><div class="path" style="white-space:pre-wrap">${esc(out || 'Finished')}</div><div class="foot"><button class="btn pri" data-close>Close</button></div>`); }
    catch (e) { delete dlg.dataset.lock; openDlg(`<h3>Failed</h3><div class="problems">${esc(e)}</div><div class="foot"><button class="btn pri" data-close>Close</button></div>`); }
    await refreshDrives(); S.sys = await invoke('system_info'); render();
  };
  if (!SYS_WARN[task]) return go();
  openDlg(`<h3>Are you sure?</h3><div class="problems">${esc(SYS_WARN[task])}</div><div class="foot"><button class="btn line" data-close>Cancel</button><button class="btn pri" id="go">Continue</button></div>`);
  $('#go').onclick = go;
}

// ---------- settings: general ----------
S.inst = null;
function generalHtml() {
  const s = S.settings, i = S.inst;
  return `<section style="margin-top:0"><h2>General</h2><div class="card">
    <div class="row" style="grid-template-columns:1fr auto"><div class="n"><b>Keep running in the tray when closed</b><small style="white-space:normal">Closing the window hides Lighten in the notification area instead of quitting, so it opens instantly. Scheduled scans do not need this; they run through Task Scheduler either way.</small></div><input type="checkbox" id="s_tray" ${s.tray_on_close ? 'checked' : ''}></div>
    <div class="row" style="grid-template-columns:1fr auto"><div class="n"><b>Dry-run report</b><small style="white-space:normal">Saves a text file on your Desktop listing every file a clean would remove, with sizes. Nothing is deleted, so it is a safe way to see what the default clean does first.</small></div><button class="btn sm line" data-ta="report">Export</button></div>
    <div class="row" style="grid-template-columns:1fr auto"><div class="n"><b>${i?.installed ? 'Installed in Program Files' : 'Run from this location'}</b><small>${i?.installed ? 'Uninstall removes the schedule, shortcut, settings and program files. Folders you moved stay linked and keep working.' : 'Install copies Lighten to Program Files, adds a Start menu entry and an Apps entry, and points the schedule at that copy. Needs administrator rights. Without it Lighten runs from where you put it.'}</small></div>${i?.installed ? '<button class="btn sm line" data-ta="uninstall">Uninstall…</button>' : '<button class="btn sm line" data-ta="install">Install…</button>'}</div></div></section>
    ${exclHtml()}`;
}
window.hooks = {};
window.onGo = async (v) => {
  if (v === 'overview') loadGrowth();
  if (v === 'apps' && !S.apps) { S.apps = null; render(); try { S.apps = await invoke('apps_list'); } catch (e) { toast(String(e), true); S.apps = []; } }
  if (v === 'system') { S.sys = null; S.startup = null; render(); loadStartup(); try { S.sys = await invoke('system_info'); } catch (e) { toast(String(e), true); } }
  if (v === 'find' && !S.profile) { try { S.profile = await invoke('profile_dir'); } catch {} }
  if (v !== 'explore' && S.fast) { S.fast = null; invoke('disk_release').catch(() => {}); }
  if (v === 'settings') { try { S.inst = await invoke('install_state'); } catch {} }
  if (v === 'clean' && S.cleanDrive !== 'C:' && !S.dcats) loadDCats();
};
Object.assign(views, { find: vFind, apps: vApps, system: vSystem });

// ---------- events ----------
document.addEventListener('click', async (e) => {
  const t = e.target.closest('[data-ft]');
  if (t) return findTab(t.dataset.ft);
  const fa = e.target.closest('[data-fa]');
  if (fa) {
    const k = fa.dataset.fa;
    if (k === 'run') return findRun();
    if (k === 'do') return findDo();
    if (k === 'all') { (S.find.res || []).forEach((h) => S.find.sel.add(h.path)); return render(); }
    if (k === 'reveal') return invoke('reveal_file', { path: fa.dataset.p });
  }
  const ap = e.target.closest('[data-ap]');
  if (ap) {
    if (ap.dataset.ap === 'move') return doMove(ap.dataset.p);
    const app = (S.apps || []).find((x) => x.id === ap.dataset.id);
    openDlg(`<h3>Uninstall ${esc(app?.name)}?</h3><p class="mut">This opens the app's own uninstaller.</p><div class="foot"><button class="btn line" data-close>Cancel</button><button class="btn pri" id="go">Open uninstaller</button></div>`);
    $('#go').onclick = async () => { try { await invoke('uninstall_app', { id: ap.dataset.id }); dlg.close(); toast('Uninstaller opened'); } catch (er) { toast(String(er), true); } };
    return;
  }
  const sa = e.target.closest('[data-sa]');
  if (sa) { if (sa.dataset.sa === 'refresh') { S.sys = null; render(); S.sys = await invoke('system_info'); return render(); } return sysRun(sa.dataset.sa, sa.dataset.p); }
  const ta = e.target.closest('[data-ta]');
  if (ta) {
    const k = ta.dataset.ta;
    if (k === 'report') { try { const p = await invoke('export_report'); toast('Saved ' + p); invoke('reveal_file', { path: p }); } catch (er) { toast(String(er), true); } return; }
    if (k === 'install') { try { await invoke('install_app'); } catch (er) { toast(String(er), true); } return; }
    if (k === 'uninstall') {
      openDlg(`<h3>Uninstall Lighten?</h3><p class="mut">Removes the schedule, shortcut, settings and program files. Folders already moved stay linked and keep working.</p><div class="foot"><button class="btn line" data-close>Cancel</button><button class="btn pri" id="go">Uninstall</button></div>`);
      $('#go').onclick = async () => { try { await invoke('uninstall_self'); } catch (er) { toast(String(er), true); } };
    }
  }
});
document.addEventListener('change', (e) => {
  const id = e.target.id;
  if (e.target.dataset.fp !== undefined) {
    const f = S.find, p = e.target.dataset.fp;
    if (e.target.checked) {
      if (f.tab === 'dupes' || f.tab === 'photos') {
        const g = f.res.find((x) => x.files.some((h) => h.path === p));
        if (g && g.files.filter((h) => h.path !== p).every((h) => f.sel.has(h.path))) { e.target.checked = false; return toast('Keep at least one copy', true); }
      }
      f.sel.add(p);
    } else f.sel.delete(p);
    return render();
  }
  if (id === 'f_perm') { S.find.perm = e.target.checked; render(); }
  if (id === 'cdrv') { setCleanDrive(e.target.value); }
});
document.addEventListener('input', (e) => { if (e.target.id === 'a_q') { S.appq = e.target.value; const pos = e.target.selectionStart; render(); const el = $('#a_q'); el.focus(); el.setSelectionRange(pos, pos); } });
