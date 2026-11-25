// Ask box: type a problem, get the page or setting that fixes it. Runs offline; it is a keyword matcher, not an AI.
const ASK = [
  { k: ['space', 'full', 'less free', 'disk', 'storage', 'dropped', 'low on', 'where did', 'gone'],
    a: 'Start with a Fast scan of C: in Explore. It shows what uses the space and flags files Windows hides. Then clean what is safe.',
    go: [['Fast scan C:', 'fast'], ['Clean page', 'view:clean'], ['Largest files', 'find:large']] },
  { k: ['hidden', 'ghost', 'chkdsk', 'cannot find', 'missing file', 'invisible', 'found.000'],
    a: 'Large files that the disk lists but Windows cannot open show up after a Fast scan, with a button that schedules a disk check for the next restart.',
    go: [['Fast scan C:', 'fast']] },
  { k: ['clean', 'temp', 'cache', 'junk', 'cleanup', 'free up'], a: 'The Clean page lists safe items with sizes. Nothing is deleted until you press Clean selected.', go: [['Open Clean', 'view:clean']] },
  { k: ['duplicate', 'same file', 'copies'], a: 'Duplicates are matched by content. The oldest copy of each is kept.', go: [['Find duplicates', 'find:dupes']] },
  { k: ['photo', 'picture', 'image', 'similar'], a: 'Similar photos finds resized or re-saved copies. Raise "Check up to" for big libraries.', go: [['Find similar photos', 'find:photos']] },
  { k: ['large', 'big file', 'biggest', 'huge'], a: 'Large files lists the biggest files in a folder or on a whole drive.', go: [['Find large files', 'find:large']] },
  { k: ['installer', 'setup', 'iso', 'msi', 'download'], a: 'Old installers in Downloads can usually go.', go: [['Find installers', 'find:installers']] },
  { k: ['node_modules', 'target', 'venv', 'dev', 'project', 'build folder'], a: 'Dev leftovers lists node_modules, Rust target and Python venv folders in projects you have not touched.', go: [['Find dev leftovers', 'find:dev']] },
  { k: ['empty folder', 'empty dir'], a: 'Empty folders lists folders with nothing inside. Nothing is pre-selected.', go: [['Find empty folders', 'find:empty']] },
  { k: ['shortcut', 'broken link', 'lnk'], a: 'Broken shortcuts finds Desktop and Start menu links whose target is gone.', go: [['Find broken shortcuts', 'find:shortcuts']] },
  { k: ['onedrive', 'cloud'], a: 'Free up space keeps OneDrive files in the cloud and removes the local copy.', go: [['OneDrive finder', 'find:onedrive']] },
  { k: ['undo', 'restore', 'deleted by mistake', 'recycle', 'got deleted', 'bring back'], a: 'Undo lists files Lighten sent to the Recycle Bin that are still in it, and puts them back.', go: [['Open Undo', 'find:undo']] },
  { k: ['move', 'another drive', 'other drive', 'd:', 'junction', 'symlink'], a: 'Move copies a folder to another drive, verifies it, and leaves a link so apps keep working.', go: [['Open Move', 'view:move']] },
  { k: ['moved', 'move back', 'undo move'], a: 'Moved folders lists everything linked to another drive and can move it back.', go: [['Open Moved folders', 'view:moved']] },
  { k: ['startup', 'slow boot', 'boot', 'starts with windows', 'autostart', 'login'], a: 'Startup apps can be turned off without uninstalling them.', go: [['Open System', 'view:system']] },
  { k: ['winsxs', 'component store', 'dism', 'windows update', 'windows.old', 'hiberfil', 'hibernat', 'restore point', 'crash', 'dump', 'disk check', 'errors', 'corrupt', 'system'], a: 'The System page has the component store report, Windows.old, hibernation, restore points and other Windows tasks.', go: [['Open System', 'view:system']] },
  { k: ['app', 'uninstall', 'program'], a: 'Apps lists installed programs by size.', go: [['Open Apps', 'view:apps']] },
  { k: ['schedule', 'automatic', 'background', 'notify', 'notification', 'every', 'auto clean', 'autoclean'], a: 'Scheduled scan is in Settings: turn it on, pick the days, and choose whether it also cleans the safe set.', go: [['Open Settings', 'view:settings']] },
  { k: ['tray', 'close button', 'minimize', 'quit'], a: 'The tray setting is under General in Settings.', go: [['Open Settings', 'view:settings']] },
  { k: ['exclude', 'protect', 'never touch', 'skip folder', 'ignore', 'keep my'], a: 'Add the folder under "Never touch these" in Settings. Every finder, Clean and Move will skip it.', go: [['Open exclusions', 'excl']] },
  { k: ['update', 'version', 'upgrade'], a: 'Updates are under Settings. Set a manifest URL, then press Check for updates.', go: [['Open Settings', 'view:settings']] },
  { k: ['dark', 'light', 'theme', 'night'], a: 'Switches between the light and dark theme.', go: [['Switch theme', 'theme']] },
  { k: ['report', 'dry run', 'preview what'], a: 'The dry-run report lists what a clean would delete, without deleting. It is in Settings under General.', go: [['Open Settings', 'view:settings']] },
  { k: ['install', 'start menu'], a: 'Install and Uninstall are in Settings under General.', go: [['Open Settings', 'view:settings']] },
  { k: ['command', 'cli', 'terminal', 'script'], a: 'From an elevated terminal: Lighten.exe --scan, --clean, --version, --help. Output can also be redirected to a file.', go: [] },
];
const ASK_TIPS = 'Try: "c drive is full", "find duplicates", "turn on dark mode", "protect a folder", "undo delete", "slow startup".';
S.ask = { open: false, log: [] };

function askMatch(q) {
  const t = q.toLowerCase();
  const hits = ASK.map((x) => ({ x, n: x.k.filter((w) => t.includes(w)).length })).filter((h) => h.n).sort((a, b) => b.n - a.n);
  return hits.slice(0, 2).map((h) => h.x);
}
async function askAct(code) {
  const [kind, arg] = code.split(':');
  S.ask.open = false; renderAsk();
  if (kind === 'view') return go(arg);
  if (kind === 'find') { await go('find'); return findTab(arg); }
  if (kind === 'excl') { await go('settings'); setTimeout(() => { const i = $('#x_in'); if (i) { i.scrollIntoView({ block: 'center' }); i.focus(); } }, 50); return; }
  if (kind === 'theme') return document.querySelector('[data-th]')?.click();
  if (kind === 'fast') { await go('explore'); return document.querySelector('[data-a="fastscan"][data-p="C:"]')?.click(); }
}
function askSend(q) {
  q = q.trim(); if (!q) return;
  S.ask.log.push({ me: true, t: q });
  const m = askMatch(q);
  if (!m.length) S.ask.log.push({ t: `I did not find a match. ${ASK_TIPS}`, go: [] });
  else m.forEach((x) => S.ask.log.push({ t: x.a, go: x.go }));
  renderAsk();
}
function closeAsk() { S.ask.open = false; renderAsk(); $('#ask .ab').focus(); }
function renderAsk() {
  let box = $('#ask');
  if (!box) { box = document.createElement('div'); box.id = 'ask'; document.body.appendChild(box); }
  const a = S.ask;
  const msgs = a.log.map((m) => m.me ? `<div class="am me">${esc(m.t)}</div>` : `<div class="am">${esc(m.t)}${m.go.length ? `<div class="ag">${m.go.map(([l, c]) => `<button class="btn sm line" data-ask-go="${c}">${esc(l)}</button>`).join('')}</div>` : ''}</div>`).join('');
  box.innerHTML = a.open
    ? `<div class="ap" role="dialog" aria-label="Ask Lighten"><div class="ah"><b>Ask Lighten</b><button class="link" data-ask="close" aria-label="Close">Close</button></div>
        <div class="al" id="ask_log">${msgs || `<div class="am">Describe a problem or what you want to change. I will point you to the page or setting. ${esc(ASK_TIPS)}</div>`}</div>
        <form class="af" data-ask="form"><input id="ask_in" placeholder="What do you need?" aria-label="Ask Lighten" autocomplete="off"><button class="btn pri" type="submit">Ask</button></form></div>`
    : `<button class="ab btn pri" data-ask="open" aria-label="Ask Lighten">Ask</button>`;
  if (a.open) { const l = $('#ask_log'); if (l) l.scrollTop = l.scrollHeight; $('#ask_in')?.focus(); }
}
document.addEventListener('click', (e) => {
  const g = e.target.closest('[data-ask-go]'); if (g) return askAct(g.dataset.askGo);
  const b = e.target.closest('[data-ask]'); if (!b) return;
  if (b.dataset.ask === 'open') { S.ask.open = true; renderAsk(); }
  if (b.dataset.ask === 'close') closeAsk();
});
document.addEventListener('submit', (e) => {
  if (!e.target.closest('[data-ask="form"]')) return;
  e.preventDefault(); const i = $('#ask_in'); const q = i.value; askSend(q);
});
document.addEventListener('keydown', (e) => { if (e.key === 'Escape' && S.ask.open) closeAsk(); });
renderAsk();
