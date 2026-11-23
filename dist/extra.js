// Theme, treemap, fast scan, startup apps, WinSxS, exclusions. Loaded last; shares globals with app.js and tools.js.
S.expMode = 'list'; S.fast = null; S.startup = null; S.sx = null;

// ---------- theme ----------
document.addEventListener('click', (e) => {
  if (!e.target.closest('[data-th]')) return;
  const next = document.documentElement.dataset.theme === 'dark' ? 'light' : 'dark';
  document.documentElement.dataset.theme = next;
  try { localStorage.setItem('theme', next); } catch {}
  if (cur === 'overview') render(); // the growth chart is drawn from CSS variables, so it follows by itself; render keeps it consistent
});

// ---------- treemap (squarified) ----------
function squarify(items, x, y, w, h) {
  const out = [];
  let rest = items.slice(), total = rest.reduce((a, i) => a + i.v, 0);
  const worst = (row, side, scale) => {
    const s = row.reduce((a, i) => a + i.v, 0) * scale, mx = Math.max(...row.map((i) => i.v)) * scale, mn = Math.min(...row.map((i) => i.v)) * scale;
    return Math.max((side * side * mx) / (s * s), (s * s) / (side * side * mn));
  };
  while (rest.length && w > 0 && h > 0) {
    const side = Math.min(w, h), scale = (w * h) / total;
    let row = [rest[0]], i = 1;
    while (i < rest.length && worst(row.concat(rest[i]), side, scale) <= worst(row, side, scale)) { row.push(rest[i]); i++; }
    const area = row.reduce((a, r) => a + r.v, 0) * scale;
    if (w >= h) { const rw = area / h; let yy = y; row.forEach((it) => { const hh = (it.v * scale) / rw; out.push({ ...it, x, y: yy, w: rw, h: hh }); yy += hh; }); x += rw; w -= rw; }
    else { const rh = area / w; let xx = x; row.forEach((it) => { const ww = (it.v * scale) / rh; out.push({ ...it, x: xx, y, w: ww, h: rh }); xx += ww; }); y += rh; h -= rh; }
    total -= row.reduce((a, r) => a + r.v, 0); rest = rest.slice(row.length);
  }
  return out;
}
function treemapHtml(items) {
  const sized = items.filter((e) => e.bytes > 0);
  if (!sized.length) return '<div class="card"><div class="empty">Nothing to draw here.</div></div>';
  const top = sized.slice(0, 40), more = sized.slice(40);
  const tot = sized.reduce((a, e) => a + e.bytes, 0), max = top[0].bytes;
  const cells = top.map((e) => ({ v: e.bytes, e }));
  if (more.length) cells.push({ v: more.reduce((a, e) => a + e.bytes, 0), rest: more.length });
  cells.sort((a, b) => b.v - a.v);
  const W = 1000, H = 560;
  const html = squarify(cells, 0, 0, W, H).map((c) => {
    const st = `left:${(c.x / W) * 100}%;top:${(c.y / H) * 100}%;width:${(c.w / W) * 100}%;height:${(c.h / H) * 100}%;--s:${(c.v / max).toFixed(3)}`;
    const roomy = c.w > 70 && c.h > 40;
    if (c.rest) return `<div class="tmc rest" style="${st}">${roomy ? `<b>${c.rest} smaller items</b><span class="mut">${txt(c.v)}</span>` : ''}</div>`;
    const inner = roomy ? `<b>${esc(c.e.name)}</b><span class="mut">${txt(c.e.bytes)} · ${Math.round((c.e.bytes / tot) * 100)}%</span>` : '';
    const tip = `${esc(c.e.name)}: ${txt(c.e.bytes)}`;
    return c.e.is_dir ? `<button class="tmc" style="${st}" data-open="${esc(c.e.path)}" title="${tip}" aria-label="${tip}. Open folder">${inner}</button>` : `<div class="tmc" style="${st}" title="${tip}">${inner}</div>`;
  }).join('');
  return `<div class="tm" role="group" aria-label="Treemap of this folder. Larger tiles use more space.">${html}</div>`;
}

// ---------- explore: fast scan and view mode ----------
document.addEventListener('click', async (e) => {
  const a = e.target.closest('[data-a]');
  if (!a) return;
  if (a.dataset.a === 'expmode') { S.expMode = S.expMode === 'map' ? 'list' : 'map'; return render(); }
  if (a.dataset.a === 'fastscan') {
    const d = a.dataset.p;
    openDlg(`<h3>Scanning ${esc(d)}</h3><p class="mut">Reading the NTFS file table. This takes a few seconds to a minute.</p>`); dlg.dataset.lock = '1';
    try {
      const r = await invoke('disk_scan', { drive: d });
      delete dlg.dataset.lock; dlg.close();
      S.fast = d.toUpperCase();
      toast(`${r.files.toLocaleString()} files in ${r.secs}s, ${txt(r.bytes)} used`);
      S.hidden = null; invoke('hidden_space').then((h) => { S.hidden = h; if (cur === 'explore') render(); }).catch(() => {});
      loadExplore(d + '\\');
    } catch (er) { delete dlg.dataset.lock; openDlg(`<h3>Fast scan unavailable</h3><div class="problems">${esc(er)}</div><p class="mut">Open the drive normally to scan it folder by folder.</p><div class="foot"><button class="btn pri" data-close>Close</button></div>`); }
  }
});

// ---------- system: WinSxS and startup apps ----------
function winsxsRow() {
  const r = S.sx;
  const detail = r ? r.map(([k, v]) => `${esc(k)}: <span style="color:var(--text)">${esc(v)}</span>`).join('<br>') : 'Windows keeps older component versions here. Explorer overstates its size; Analyze asks Windows for the real figure. Takes about a minute.';
  const btns = r ? '<button class="btn sm line" data-sx="analyze">Analyze again</button><button class="btn sm line" data-sx="clean">Clean up…</button>' : '<button class="btn sm line" data-sx="analyze">Analyze</button>';
  return sysRow('Component store (WinSxS)', detail, btns);
}
async function loadStartup() { try { S.startup = await invoke('startup_list'); } catch { S.startup = []; } if (cur === 'system') render(); }
function startupHtml() {
  const l = S.startup;
  const rows = !l ? '' : l.map((s) => `<div class="row" style="grid-template-columns:1fr 90px 100px"><div class="n"><b>${esc(s.name)}</b><small title="${esc(s.command)}">${esc(s.command)}</small></div><div><span class="pill ${s.enabled ? 'ok' : ''}">${s.enabled ? 'On' : 'Off'}</span> <span class="mut" style="font-size:12px">${s.scope === 'HKCU' ? 'You' : 'All users'}</span></div><div class="r"><button class="btn sm line" data-su="${esc(s.id)}" data-en="${s.enabled ? '0' : '1'}">${s.enabled ? 'Turn off' : 'Turn on'}</button></div></div>`).join('') || '<div class="empty">No startup entries found.</div>';
  return `<section><h2>Startup apps <span>Turning one off keeps the app installed; it just stops starting with Windows</span></h2><div class="card">${l ? rows : skeleton()}</div></section>`;
}
document.addEventListener('click', async (e) => {
  const sx = e.target.closest('[data-sx]');
  if (sx) {
    if (sx.dataset.sx === 'analyze') {
      openDlg('<h3>Analyzing component store…</h3><p class="mut">Windows is measuring it. About a minute.</p>'); dlg.dataset.lock = '1';
      try { S.sx = await invoke('winsxs_analyze'); delete dlg.dataset.lock; dlg.close(); } catch (er) { delete dlg.dataset.lock; openDlg(`<h3>Could not analyze</h3><div class="problems">${esc(er)}</div><div class="foot"><button class="btn pri" data-close>Close</button></div>`); }
      return render();
    }
    openDlg('<h3>Clean up the component store?</h3><p class="mut">Runs Windows\' own cleanup of superseded component versions. It can take several minutes, and already installed updates can no longer be uninstalled afterwards.</p><div class="foot"><button class="btn line" data-close>Cancel</button><button class="btn pri" id="go">Clean up</button></div>');
    $('#go').onclick = async () => {
      dlg.dataset.lock = '1'; openDlg('<h3>Cleaning up…</h3><p class="mut">Keep Lighten open.</p>');
      try { await invoke('admin_task', { task: 'dism' }); await refreshDrives(); S.sx = null; delete dlg.dataset.lock; dlg.close(); toast('Component store cleaned'); }
      catch (er) { delete dlg.dataset.lock; openDlg(`<h3>Failed</h3><div class="problems">${esc(er)}</div><div class="foot"><button class="btn pri" data-close>Close</button></div>`); }
      render();
    };
    return;
  }
  const su = e.target.closest('[data-su]');
  if (su) {
    su.disabled = true;
    try { await invoke('startup_set', { id: su.dataset.su, enabled: su.dataset.en === '1' }); } catch (er) { toast(String(er), true); }
    loadStartup();
  }
});

// ---------- settings: exclusions ----------
function exclHtml() {
  const l = (S.settings && S.settings.exclusions) || [];
  const rows = l.map((p, i) => `<div class="row" style="grid-template-columns:1fr auto"><div class="n"><b style="font-weight:400">${esc(p)}</b></div><button class="btn sm line" data-x="del" data-i="${i}">Remove</button></div>`).join('');
  return `<section><h2>Never touch these <span>Finders, Clean and Move skip these folders</span></h2><div class="card">${rows || '<div class="empty">No exclusions.</div>'}
    <div class="row" style="grid-template-columns:1fr"><div class="n"><small style="white-space:normal">Add a folder here and Lighten will not list it, delete from it, clean it or move it, including everything inside. Use it for projects, game saves or anything you want left alone. Matching is by whole folder name, so C:\\Work does not cover C:\\Workshop.</small></div></div>
    <div class="excl"><input class="t" id="x_in" placeholder="C:\\Users\\you\\Projects" aria-label="Folder to exclude"><button class="btn line" data-x="add">Add</button></div></div></section>`;
}
document.addEventListener('click', async (e) => {
  const x = e.target.closest('[data-x]');
  if (!x) return;
  const cur_ = (S.settings && S.settings.exclusions) || [];
  let next = cur_.slice();
  if (x.dataset.x === 'add') { const v = ($('#x_in').value || '').trim(); if (!v) return; next.push(v); }
  else next.splice(+x.dataset.i, 1);
  try { S.settings.exclusions = await invoke('set_exclusions', { list: next }); } catch (er) { toast(String(er), true); }
  render();
});

// ---------- hidden space: files Windows lists as missing but that still use the disk ----------
S.hidden = null;
function hiddenHtml() {
  const h = S.hidden;
  if (!h || !h.length) return '';
  const tot = h.reduce((a, x) => a + x.bytes, 0);
  const list = h.slice(0, 5).map((x) => `<div><code>${esc(x.path)}</code> · ${txt(x.bytes)}</div>`).join('');
  return `<div class="alert"><b>${txt(tot)} is used by ${h.length} file(s) Windows cannot find.</b> The disk's file table lists them, but opening them by name fails, so no folder shows them and Explorer cannot delete them.
    <div style="margin:8px 0">${list}</div>
    A disk check at the next restart can reattach them to a <code>found.000</code> folder, where you can delete them. <button class="btn sm line" data-a="chkdsk" data-p="${esc(S.fast || 'C:')}">Schedule disk check…</button></div>`;
}
document.addEventListener('click', async (e) => {
  const a = e.target.closest('[data-a="chkdsk"]');
  if (!a) return;
  const d = a.dataset.p;
  openDlg(`<h3>Check ${esc(d)} at next restart?</h3><p class="mut">Windows will run <code>chkdsk ${esc(d)} /f</code> before it starts. It repairs file-system errors and can take from a few minutes to an hour on large drives; the PC is unusable until it finishes. Files it recovers go to <code>${esc(d)}\found.000</code>. Nothing is deleted without you.</p>
    <div class="foot"><button class="btn line" data-close>Cancel</button><button class="btn pri" id="go">Schedule</button></div>`);
  $('#go').onclick = async () => {
    $('#go').disabled = true;
    try { const r = await invoke('schedule_chkdsk', { drive: d }); dlg.close(); toast(/next time|restart/i.test(r) ? 'Scheduled. Restart to run it.' : r || 'Scheduled'); }
    catch (er) { dlg.close(); toast(String(er), true); }
  };
});
