#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! Lighten backend. Safety model:
//! - Cleaning only touches fixed allowlisted roots (never a path from the UI) and never follows reparse points.
//! - Moving copies, verifies counts+bytes, renames the original aside, links, verifies, and only then deletes. Every step rolls back.

mod tools;
use serde::{Deserialize, Serialize};
use std::os::windows::{fs::MetadataExt, process::CommandExt};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};
use sysinfo::{Disks, ProcessesToUpdate, System};
use tauri::{AppHandle, Emitter};

const NO_WINDOW: u32 = 0x0800_0000;
const REPARSE: u32 = 0x400;

fn envp(k: &str) -> PathBuf {
    PathBuf::from(std::env::var(k).unwrap_or_default())
}
fn is_reparse(md: &fs::Metadata) -> bool {
    md.file_attributes() & REPARSE != 0
}
fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
fn run(cmd: &str, args: &[&str]) -> Result<std::process::Output, String> {
    Command::new(cmd).args(args).creation_flags(NO_WINDOW).output().map_err(|e| format!("{cmd}: {e}"))
}
fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> impl std::future::Future<Output = Result<T, String>> {
    async move { tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string()) }
}

// ---------- sizing ----------
#[derive(Default, Clone, Copy, Serialize)]
struct Sz {
    bytes: u64,
    files: u64,
    links: u64,
}
/// Sums files under `root`; reparse points are counted, never entered. Files younger than `min_age` secs are ignored.
fn dir_size(root: &Path, min_age: u64) -> Sz {
    let mut t = Sz::default();
    let n = SystemTime::now();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(md) = fs::symlink_metadata(&p) else { continue };
            if is_reparse(&md) {
                t.links += 1;
            } else if md.is_dir() {
                stack.push(p);
            } else {
                if min_age > 0 {
                    if let Ok(m) = md.modified() {
                        if n.duration_since(m).map(|a| a.as_secs()).unwrap_or(0) < min_age {
                            continue;
                        }
                    }
                }
                t.bytes += md.len();
                t.files += 1;
            }
        }
    }
    t
}
/// Sizes many dirs in parallel (bounded).
fn sizes_parallel(paths: &[PathBuf], min_age: u64) -> Vec<Sz> {
    let mut out = vec![Sz::default(); paths.len()];
    let chunk = (paths.len() / 8).max(1);
    std::thread::scope(|s| {
        for (ps, os) in paths.chunks(chunk).zip(out.chunks_mut(chunk)) {
            s.spawn(move || {
                for (p, o) in ps.iter().zip(os.iter_mut()) {
                    *o = dir_size(p, min_age);
                }
            });
        }
    });
    out
}

fn running_procs() -> Vec<(String, Option<PathBuf>)> {
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    sys.processes()
        .values()
        .map(|p| (p.name().to_string_lossy().to_lowercase().trim_end_matches(".exe").to_string(), p.exe().map(|e| e.to_path_buf())))
        .collect()
}
fn lc(p: &Path) -> String {
    p.to_string_lossy().replace('/', "\\").to_lowercase().trim_end_matches('\\').to_string()
}
/// Case-insensitive prefix strip on whole path components.
fn strip_ci(path: &Path, base: &Path) -> Option<PathBuf> {
    let b = lc(base);
    let p = lc(path);
    if p == b {
        return Some(PathBuf::new());
    }
    if p.starts_with(&(b.clone() + "\\")) {
        let orig = path.to_string_lossy().replace('/', "\\");
        let orig = orig.trim_end_matches('\\');
        return Some(PathBuf::from(&orig[b.len() + 1..]));
    }
    None
}

// ---------- drives ----------
#[derive(Serialize)]
struct Drive {
    letter: String,
    label: String,
    total: u64,
    free: u64,
    removable: bool,
}
#[tauri::command]
fn drives() -> Vec<Drive> {
    Disks::new_with_refreshed_list()
        .iter()
        .map(|d| Drive {
            letter: d.mount_point().to_string_lossy().trim_end_matches('\\').to_string(),
            label: d.name().to_string_lossy().to_string(),
            total: d.total_space(),
            free: d.available_space(),
            removable: d.is_removable(),
        })
        .filter(|d| d.total > 0)
        .collect()
}

// ---------- cleaning ----------
struct Cat {
    id: &'static str,
    name: &'static str,
    desc: &'static str,
    roots: Vec<PathBuf>,
    min_age: u64,
    admin: bool,
    procs: &'static [&'static str],
    default_on: bool,
}
fn browser_roots(base: PathBuf) -> Vec<PathBuf> {
    let mut v = vec![];
    if let Ok(rd) = fs::read_dir(&base) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n == "Default" || n.starts_with("Profile ") {
                for s in ["Cache", "Code Cache", "GPUCache"] {
                    let p = e.path().join(s);
                    if p.is_dir() {
                        v.push(p);
                    }
                }
            }
        }
    }
    v
}
fn cats() -> Vec<Cat> {
    let local = envp("LOCALAPPDATA");
    let prof = envp("USERPROFILE");
    let win = envp("SystemRoot");
    let pd = envp("ProgramData");
    let roam = envp("APPDATA");
    vec![
        Cat { id: "nvupd", name: "NVIDIA App update downloads", desc: "Installer packages the NVIDIA App keeps. Re-downloaded unless you turn off its auto-update.", roots: vec![pd.join(r"NVIDIA Corporation\NVIDIA App\UpdateFramework")], min_age: 0, admin: false, procs: &[], default_on: true },
        Cat { id: "discord", name: "Discord cache", desc: "Cached images and code. Your login and messages are untouched.", roots: vec![roam.join(r"discord\Cache"), roam.join(r"discord\Code Cache"), roam.join(r"discord\GPUCache")], min_age: 0, admin: false, procs: &["discord"], default_on: false },
        Cat { id: "store", name: "Microsoft Store cache", desc: "Store app cache. Rebuilt automatically.", roots: vec![local.join(r"Packages\Microsoft.WindowsStore_8wekyb3d8bbwe\LocalCache")], min_age: 0, admin: false, procs: &["winstore.app"], default_on: true },
        Cat { id: "winlogs", name: "Old Windows logs", desc: "Log files older than a week in C:\\Windows\\Logs.", roots: vec![win.join("Logs")], min_age: 7 * 86400, admin: false, procs: &[], default_on: true },
        Cat { id: "temp", name: "User temp files", desc: "Leftovers apps wrote to %TEMP%. Files from the last hour are kept.", roots: vec![envp("TEMP")], min_age: 3600, admin: false, procs: &[], default_on: true },
        Cat { id: "inet", name: "Windows web cache", desc: "Internet cache used by Windows components.", roots: vec![local.join(r"Microsoft\Windows\INetCache")], min_age: 3600, admin: false, procs: &[], default_on: true },
        Cat { id: "dumps", name: "Crash dumps and error reports", desc: "Memory dumps and Windows Error Reporting data.", roots: vec![local.join("CrashDumps"), local.join(r"Microsoft\Windows\WER"), pd.join(r"Microsoft\Windows\WER")], min_age: 3600, admin: false, procs: &[], default_on: true },
        Cat { id: "shader", name: "Shader caches", desc: "DirectX, NVIDIA and AMD shader caches. Games rebuild them once, briefly slower.", roots: vec![local.join("D3DSCache"), local.join(r"NVIDIA\DXCache"), local.join(r"NVIDIA\GLCache"), local.join(r"AMD\DxCache")], min_age: 0, admin: false, procs: &[], default_on: true },
        Cat { id: "npm", name: "npm cache", desc: "Downloaded packages. Re-fetched when needed.", roots: vec![local.join("npm-cache")], min_age: 0, admin: false, procs: &["npm", "node"], default_on: true },
        Cat { id: "pip", name: "pip cache", desc: "Downloaded Python wheels. Re-fetched when needed.", roots: vec![local.join(r"pip\Cache")], min_age: 0, admin: false, procs: &["pip", "python"], default_on: true },
        Cat { id: "nuget", name: "NuGet cache", desc: "Downloaded .NET packages. Re-fetched when needed.", roots: vec![local.join(r"NuGet\v3-cache")], min_age: 0, admin: false, procs: &["dotnet", "devenv"], default_on: true },
        Cat { id: "cargo", name: "Cargo download cache", desc: "Crate archives and sources. Re-downloaded on the next build.", roots: vec![prof.join(r".cargo\registry\cache"), prof.join(r".cargo\registry\src")], min_age: 0, admin: false, procs: &["cargo", "rustc"], default_on: false },
        Cat { id: "chrome", name: "Chrome cache", desc: "Cached pages and code. Logins, bookmarks and history are untouched.", roots: browser_roots(local.join(r"Google\Chrome\User Data")), min_age: 0, admin: false, procs: &["chrome"], default_on: false },
        Cat { id: "edge", name: "Edge cache", desc: "Cached pages and code. Logins, bookmarks and history are untouched.", roots: browser_roots(local.join(r"Microsoft\Edge\User Data")), min_age: 0, admin: false, procs: &["msedge"], default_on: false },
        Cat { id: "brave", name: "Brave cache", desc: "Cached pages and code. Logins, bookmarks and history are untouched.", roots: browser_roots(local.join(r"BraveSoftware\Brave-Browser\User Data")), min_age: 0, admin: false, procs: &["brave"], default_on: false },
        Cat { id: "wutemp", name: "Windows Update download cache", desc: "Already-installed update packages. ", roots: vec![win.join(r"SoftwareDistribution\Download")], min_age: 0, admin: false, procs: &[], default_on: true },
        Cat { id: "do", name: "Delivery Optimization cache", desc: "Windows update files shared with other PCs. ", roots: vec![pd.join(r"Microsoft\Windows\DeliveryOptimization\Cache")], min_age: 0, admin: false, procs: &[], default_on: true },
        Cat { id: "wintemp", name: "Windows temp files", desc: "System-wide temp folder. ", roots: vec![win.join("Temp")], min_age: 3600, admin: false, procs: &[], default_on: true },
    ]
}

#[derive(Serialize)]
struct CatInfo {
    id: String,
    name: String,
    desc: String,
    bytes: u64,
    files: u64,
    admin: bool,
    busy: Option<String>,
    default_on: bool,
}
fn recycle_bytes() -> u64 {
    let ps = "(New-Object -ComObject Shell.Application).NameSpace(0xA).Items() | ForEach-Object { $_.Size } | Measure-Object -Sum | ForEach-Object { [int64]$_.Sum }";
    run("powershell", &["-NoProfile", "-Command", ps])
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u64>().ok())
        .unwrap_or(0)
}
fn busy_of(c: &Cat, procs: &[(String, Option<PathBuf>)]) -> Option<String> {
    c.procs.iter().find(|p| procs.iter().any(|(n, _)| n == *p)).map(|p| p.to_string())
}

fn drive_letter(d: &str) -> Option<char> {
    let c = d.trim().chars().next()?.to_ascii_uppercase();
    if c.is_ascii_uppercase() && drives().iter().any(|x| x.letter.to_ascii_uppercase().starts_with(c)) { Some(c) } else { None }
}
fn drive_cats(l: char) -> Vec<CatInfo> {
    let mut v = vec![];
    for (id, rel) in [("dtemp", "Temp"), ("dtmp", "tmp")] {
        let p = PathBuf::from(format!("{l}:\\{rel}"));
        if p.is_dir() {
            let s = dir_size(&p, 3600);
            v.push(CatInfo { id: id.into(), name: format!("{l}:\\{rel}"), desc: "Temporary files older than an hour.".into(), bytes: s.bytes, files: s.files, admin: false, busy: None, default_on: true });
        }
    }
    let rb = PathBuf::from(format!("{l}:\\$Recycle.Bin"));
    v.push(CatInfo { id: "recycle".into(), name: format!("Recycle Bin on {l}:"), desc: "Permanently deletes what is in this drive's Recycle Bin. Cannot be undone.".into(), bytes: dir_size(&rb, 0).bytes, files: 0, admin: false, busy: None, default_on: false });
    v
}
fn clean_drive(l: char, ids: Vec<String>) -> Vec<CleanResult> {
    let mut out = vec![];
    for id in ids {
        let mut r = CleanResult { id: id.clone(), ..Default::default() };
        match id.as_str() {
            "dtemp" | "dtmp" => {
                let p = PathBuf::from(format!("{l}:\\{}", if id == "dtemp" { "Temp" } else { "tmp" }));
                if p.is_dir() { clean_dir(&p, 3600, &mut r); }
            }
            "recycle" => {
                let rb = PathBuf::from(format!("{l}:\\$Recycle.Bin"));
                let before = dir_size(&rb, 0).bytes;
                let _ = run("powershell", &["-NoProfile", "-Command", &format!("Clear-RecycleBin -DriveLetter {l} -Force -ErrorAction SilentlyContinue")]);
                r.freed = before.saturating_sub(dir_size(&rb, 0).bytes);
            }
            _ => r.note = Some("Unknown category".into()),
        }
        out.push(r);
    }
    out
}
fn scan_cats(drive: &str) -> Vec<CatInfo> {
    if !drive.trim().is_empty() {
        match drive_letter(drive) { Some(l) if l != 'C' => return drive_cats(l), Some(_) => {}, None => return vec![] }
    }
    {
        let procs = running_procs();
        let cs = cats();
        let mut out: Vec<CatInfo> = vec![];
        for c in &cs {
            let existing: Vec<PathBuf> = c.roots.iter().filter(|r| r.is_dir()).cloned().collect();
            let sz = sizes_parallel(&existing, c.min_age);
            out.push(CatInfo {
                id: c.id.into(), name: c.name.into(), desc: c.desc.into(),
                bytes: sz.iter().map(|s| s.bytes).sum(), files: sz.iter().map(|s| s.files).sum(),
                admin: c.admin, busy: busy_of(c, &procs), default_on: c.default_on,
            });
        }
        out.push(CatInfo { id: "recycle".into(), name: "Recycle Bin".into(), desc: "Permanently deletes what is in the Recycle Bin. Cannot be undone.".into(), bytes: recycle_bytes(), files: 0, admin: false, busy: None, default_on: false });
        out
    }
}
#[tauri::command]
async fn scan_clean(drive: Option<String>) -> Result<Vec<CatInfo>, String> {
    blocking(move || scan_cats(&drive.unwrap_or_default())).await
}

#[derive(Serialize, Default)]
struct CleanResult {
    id: String,
    freed: u64,
    files: u64,
    failed: u64,
    skipped: u64,
    note: Option<String>,
}
fn clean_dir(dir: &Path, min_age: u64, r: &mut CleanResult) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let n = SystemTime::now();
    for e in rd.flatten() {
        let p = e.path();
        let Ok(md) = fs::symlink_metadata(&p) else { continue };
        if is_reparse(&md) {
            r.skipped += 1; // never follow or delete links
        } else if md.is_dir() {
            clean_dir(&p, min_age, r);
            let _ = fs::remove_dir(&p); // only succeeds when empty
        } else {
            if min_age > 0 {
                if let Ok(m) = md.modified() {
                    if n.duration_since(m).map(|a| a.as_secs()).unwrap_or(0) < min_age {
                        r.skipped += 1;
                        continue;
                    }
                }
            }
            let mut res = fs::remove_file(&p);
            if res.is_err() {
                let mut perm = md.permissions();
                perm.set_readonly(false);
                let _ = fs::set_permissions(&p, perm);
                res = fs::remove_file(&p);
            }
            match res {
                Ok(_) => { r.freed += md.len(); r.files += 1; }
                Err(_) => r.failed += 1,
            }
        }
    }
}
fn do_clean(ids: Vec<String>, drive: &str) -> Vec<CleanResult> {
    if !drive.trim().is_empty() {
        match drive_letter(drive) { Some(l) if l != 'C' => return clean_drive(l, ids), Some(_) => {}, None => return vec![] }
    }
    {
        let procs = running_procs();
        let mut out = vec![];
        for id in ids {
            let mut r = CleanResult { id: id.clone(), ..Default::default() };
            if id == "recycle" {
                let before = recycle_bytes();
                let _ = run("powershell", &["-NoProfile", "-Command", "Clear-RecycleBin -Force -ErrorAction SilentlyContinue"]);
                r.freed = before.saturating_sub(recycle_bytes());
            } else if let Some(c) = cats().into_iter().find(|c| c.id == id) {
                if let Some(p) = busy_of(&c, &procs) {
                    r.note = Some(format!("Skipped: {p} is running"));
                } else {
                    for root in c.roots.iter().filter(|x| x.is_dir()) {
                        clean_dir(root, c.min_age, &mut r);
                    }
                }
            } else {
                r.note = Some("Unknown category".into());
            }
            out.push(r);
        }
        out
    }
}
#[tauri::command]
async fn clean(ids: Vec<String>, drive: Option<String>) -> Result<Vec<CleanResult>, String> {
    blocking(move || do_clean(ids, &drive.unwrap_or_default())).await
}

// ---------- explore ----------
#[derive(Serialize)]
struct Entry {
    path: String,
    name: String,
    is_dir: bool,
    link: bool,
    bytes: u64,
    files: u64,
}
#[tauri::command]
async fn list_dir(path: String) -> Result<Vec<Entry>, String> {
    blocking(move || {
        let base = PathBuf::from(&path);
        let rd = fs::read_dir(&base).map_err(|e| e.to_string())?;
        let mut items: Vec<(PathBuf, bool, bool, u64)> = vec![];
        for e in rd.flatten() {
            let p = e.path();
            let Ok(md) = fs::symlink_metadata(&p) else { continue };
            items.push((p, md.is_dir() && !is_reparse(&md), is_reparse(&md), md.len()));
        }
        let dirs: Vec<PathBuf> = items.iter().filter(|i| i.1).map(|i| i.0.clone()).collect();
        let sz = sizes_parallel(&dirs, 0);
        let mut si = sz.into_iter();
        let mut out: Vec<Entry> = items
            .into_iter()
            .map(|(p, is_dir, link, len)| {
                let s = if is_dir { si.next().unwrap_or_default() } else { Sz { bytes: if link { 0 } else { len }, files: 1, links: 0 } };
                Entry { name: p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), path: p.to_string_lossy().to_string(), is_dir, link, bytes: s.bytes, files: s.files }
            })
            .collect();
        out.sort_by(|a, b| b.bytes.cmp(&a.bytes));
        Ok(out)
    })
    .await?
}

// ---------- move ----------
const DENY: &[&str] = &[
    "microsoft", "packages", "temp", "connecteddevices", "placeholdertilelogofolder", "comms", "publishers", "virtualstore",
    "nvidia", "nvidia corporation", "intel", "amd", "package cache", "windowsapps", "appdata", ".ssh", ".gnupg", "onedrive",
    "ntuser.dat", "common files", "windows defender", "windows nt", "windows mail", "windows media player", "windows photo viewer", "windows security", "windowspowershell", "internet explorer", "modifiableapps", "msbuild", "reference assemblies", "dotnet", "microsoft visual studio", "desktop", "documents", "downloads", "pictures", "music", "videos", "contacts", "favorites", "links", "saved games",
];
const CAUTION: &[(&str, &str)] = &[
    ("discord", "Self-updater checks its own folder; works with a link but verify after the next update"),
    ("slack", "Self-updater; verify after the next update"),
    ("teams", "Self-updater; verify after the next update"),
    ("programs", "Contains installed apps; move one app folder, not this"),
    (".claude", "Used by Claude Code; close every Claude session first"),
    ("claude", "Used by Claude; close it first"),
    ("claude-data", "Used by Claude; close it first"),
];
fn denied(p: &Path) -> bool {
    p.components().any(|c| DENY.contains(&c.as_os_str().to_string_lossy().to_lowercase().as_str()))
}
/// Names of running processes that live in, or are named like, `folder`.
fn busy_in(folder: &Path, procs: &[(String, Option<PathBuf>)]) -> Vec<String> {
    let fl = lc(folder);
    let leaf = folder.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mut v: Vec<String> = vec![];
    for (n, exe) in procs {
        let in_dir = exe.as_ref().map(|e| lc(e).starts_with(&(fl.clone() + "\\"))).unwrap_or(false);
        let by_name = n.len() >= 3 && (leaf == *n || leaf.starts_with(n.as_str()) || n.starts_with(leaf.as_str()) && leaf.len() >= 3);
        if (in_dir || by_name) && !v.contains(n) {
            v.push(n.clone());
        }
    }
    v
}

#[derive(Serialize)]
struct Cand {
    path: String,
    name: String,
    area: String,
    bytes: u64,
    files: u64,
    status: String, // ready | caution | busy
    note: Option<String>,
}
#[tauri::command]
async fn move_candidates() -> Result<Vec<Cand>, String> {
    blocking(|| {
        let prof = envp("USERPROFILE");
        let areas = [(envp("LOCALAPPDATA"), "AppData\\Local"), (envp("APPDATA"), "AppData\\Roaming"), (prof.clone(), "Profile")];
        let mut paths = vec![];
        for (base, area) in &areas {
            if let Ok(rd) = fs::read_dir(base) {
                for e in rd.flatten() {
                    let p = e.path();
                    let Ok(md) = fs::symlink_metadata(&p) else { continue };
                    if md.is_dir() && !is_reparse(&md) && !denied(&p) && !(*area == "Profile" && p.file_name().map(|n| n.to_string_lossy() == "AppData").unwrap_or(false)) {
                        paths.push((p, area.to_string()));
                    }
                }
            }
        }
        let ps: Vec<PathBuf> = paths.iter().map(|x| x.0.clone()).collect();
        let sz = sizes_parallel(&ps, 0);
        let procs = running_procs();
        let mut out: Vec<Cand> = vec![];
        for ((p, area), s) in paths.into_iter().zip(sz) {
            if s.bytes < 100 * 1024 * 1024 || s.links > 0 {
                continue; // too small, or contains links (we refuse to move those)
            }
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let busy = busy_in(&p, &procs);
            let caution = CAUTION.iter().find(|(k, _)| name.to_lowercase() == *k);
            let (status, note) = if !busy.is_empty() {
                ("busy", Some(format!("Close {} first", busy.join(", "))))
            } else if let Some((_, why)) = caution {
                ("caution", Some(why.to_string()))
            } else {
                ("ready", None)
            };
            out.push(Cand { path: p.to_string_lossy().to_string(), name, area, bytes: s.bytes, files: s.files, status: status.into(), note });
        }
        out.sort_by(|a, b| b.bytes.cmp(&a.bytes));
        out
    })
    .await
}

#[derive(Serialize, Deserialize, Clone)]
struct MoveRec {
    src: String,
    dest: String,
    bytes: u64,
    when: u64,
}
fn journal_path() -> PathBuf {
    let d = envp("LOCALAPPDATA").join("Lighten");
    let _ = fs::create_dir_all(&d);
    d.join("moves.json")
}
fn load_journal() -> Vec<MoveRec> {
    fs::read_to_string(journal_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}
fn save_journal(v: &[MoveRec]) -> Result<(), String> {
    fs::write(journal_path(), serde_json::to_string_pretty(v).unwrap()).map_err(|e| e.to_string())
}

fn robocopy(src: &Path, dest: &Path) -> Result<(), String> {
    let o = Command::new("robocopy")
        .arg(src).arg(dest)
        .args(["/E", "/COPY:DAT", "/DCOPY:DAT", "/R:1", "/W:1", "/NFL", "/NDL", "/NJH", "/NJS", "/NP"])
        .creation_flags(NO_WINDOW).output().map_err(|e| e.to_string())?;
    match o.status.code() {
        Some(c) if c < 8 => Ok(()),
        c => Err(format!("robocopy failed (code {c:?})")),
    }
}
fn same(a: Sz, b: Sz) -> bool {
    a.bytes == b.bytes && a.files == b.files
}
fn step(app: &AppHandle, key: &str, text: &str) {
    let _ = app.emit("move-step", serde_json::json!({ "key": key, "text": text }));
}
fn mk_junction(link: &Path, target: &Path) -> Result<(), String> {
    let o = Command::new("cmd").arg("/C").arg("mklink").arg("/J").arg(link).arg(target).creation_flags(NO_WINDOW).output().map_err(|e| e.to_string())?;
    if o.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&o.stdout).to_string() + &String::from_utf8_lossy(&o.stderr)) }
}

#[derive(Serialize)]
struct MovePlan {
    src: String,
    dest: String,
    bytes: u64,
    files: u64,
    dest_free: u64,
    problems: Vec<String>,
}
fn plan(src: &str, drive: &str) -> Result<MovePlan, String> {
    let src = PathBuf::from(src);
    let prof = envp("USERPROFILE");
    let mut problems = vec![];
    let mut rel = strip_ci(&src, &prof);
    if src.components().any(|c| c.as_os_str() == "..") || !src.is_absolute() {
        return Err("Path must be absolute".into());
    }
    for b in [envp("ProgramFiles"), envp("ProgramFiles(x86)")] {
        if let Some(r) = strip_ci(&src, &b) {
            if r.components().count() == 1 {
                rel = Some(PathBuf::from(b.file_name().unwrap_or_default()).join(r));
            }
        }
    }
    let rel = match rel {
        Some(r) if r.components().count() >= 1 => r,
        _ => return Err("Only folders in your profile or directly inside Program Files can be moved".into()),
    };
    if denied(&src) {
        problems.push("This folder is protected and cannot be moved".into());
    }
    let md = fs::symlink_metadata(&src).map_err(|e| format!("Cannot read folder: {e}"))?;
    if !md.is_dir() { problems.push("Not a folder".into()); }
    if is_reparse(&md) { problems.push("Already a link".into()); }
    let sd = lc(&src).chars().next().unwrap_or('c');
    let dd = drive.trim_end_matches(['\\', ':']).to_lowercase().chars().next().unwrap_or('c');
    if sd == dd { problems.push("Destination must be a different drive".into()); }
    let root = PathBuf::from(format!("{}:\\", dd.to_ascii_uppercase()));
    let d = drives();
    let free = d.iter().find(|x| x.letter.to_lowercase().starts_with(dd)).map(|x| x.free).unwrap_or(0);
    if free == 0 { problems.push("Destination drive not found".into()); }
    let dest = root.join("Lighten").join(&rel);
    if dest.exists() && fs::read_dir(&dest).map(|mut r| r.next().is_some()).unwrap_or(true) {
        problems.push("Destination already exists and is not empty".into());
    }
    let sz = dir_size(&src, 0);
    if sz.links > 0 { problems.push(format!("Contains {} link(s) inside; refusing to move", sz.links)); }
    if sz.bytes + sz.bytes / 20 > free { problems.push("Not enough free space on destination".into()); }
    let busy = busy_in(&src, &running_procs());
    if !busy.is_empty() { problems.push(format!("Still running: {}. Close it first", busy.join(", "))); }
    if rel.starts_with("Program Files") {
        let sv = tools::services_in(&src);
        if !sv.is_empty() { problems.push(format!("A service from this app is running: {}. Stop it first", sv.join(", "))); }
    }
    if load_journal().iter().any(|m| lc(Path::new(&m.src)) == lc(&src)) { problems.push("Already moved by Lighten".into()); }
    Ok(MovePlan { src: src.to_string_lossy().into(), dest: dest.to_string_lossy().into(), bytes: sz.bytes, files: sz.files, dest_free: free, problems })
}
#[tauri::command]
async fn plan_move(src: String, drive: String) -> Result<MovePlan, String> {
    blocking(move || plan(&src, &drive)).await?
}

#[tauri::command]
async fn move_folder(app: AppHandle, src: String, drive: String) -> Result<MoveRec, String> {
    blocking(move || -> Result<MoveRec, String> {
        let p = plan(&src, &drive)?;
        if !p.problems.is_empty() {
            return Err(p.problems.join("; "));
        }
        let (srcp, dest) = (PathBuf::from(&p.src), PathBuf::from(&p.dest));
        let before = dir_size(&srcp, 0);
        let mut old_name = srcp.file_name().unwrap().to_os_string();
        old_name.push(".lighten-old");
        let old = srcp.with_file_name(old_name);
        if old.exists() { return Err("A leftover .lighten-old folder exists next to the source; remove it first".into()); }

        step(&app, "copy", &format!("Copying {} files", before.files));
        fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
        let abort_copy = |msg: String| -> String { let _ = fs::remove_dir_all(&dest); msg };
        if let Err(e) = robocopy(&srcp, &dest) { return Err(abort_copy(e)); }

        step(&app, "verify", "Checking file count and size match");
        let after = dir_size(&dest, 0);
        if !same(before, after) {
            return Err(abort_copy(format!("Copy check failed ({} vs {} files). Nothing changed.", before.files, after.files)));
        }

        step(&app, "swap", "Setting the original aside");
        if let Err(e) = fs::rename(&srcp, &old) {
            return Err(abort_copy(format!("Folder is in use, original left untouched ({e})")));
        }
        step(&app, "link", "Creating the link");
        if let Err(e) = mk_junction(&srcp, &dest) {
            let _ = fs::rename(&old, &srcp);
            return Err(abort_copy(format!("Link failed, original restored ({e})")));
        }
        let linked = fs::symlink_metadata(&srcp).map(|m| is_reparse(&m)).unwrap_or(false) && same(before, dir_size(&srcp, 0));
        if !linked {
            let _ = fs::remove_dir(&srcp);
            let _ = fs::rename(&old, &srcp);
            return Err(abort_copy("Link check failed, original restored".into()));
        }
        step(&app, "clean", "Removing the old copy");
        let leftover = fs::remove_dir_all(&old).is_err() && old.exists();

        let rec = MoveRec { src: p.src, dest: p.dest, bytes: before.bytes, when: now() };
        let mut j = load_journal();
        j.push(rec.clone());
        save_journal(&j)?;
        if leftover { return Err(format!("Moved, but could not remove the old copy at {}. Delete it manually.", old.display())); }
        Ok(rec)
    })
    .await?
}

#[derive(Serialize)]
struct Moved {
    rec: MoveRec,
    status: String, // ok | broken | unlinked
}
#[tauri::command]
fn moved_list() -> Vec<Moved> {
    load_journal()
        .into_iter()
        .map(|rec| {
            let linked = fs::symlink_metadata(&rec.src).map(|m| is_reparse(&m)).unwrap_or(false);
            let target = Path::new(&rec.dest).is_dir();
            let status = if linked && target { "ok" } else if !target { "broken" } else { "unlinked" };
            Moved { rec, status: status.into() }
        })
        .collect()
}

#[tauri::command]
async fn move_back(app: AppHandle, src: String) -> Result<(), String> {
    blocking(move || -> Result<(), String> {
        let mut j = load_journal();
        let rec = j.iter().find(|m| lc(Path::new(&m.src)) == lc(Path::new(&src))).cloned().ok_or("Not in Lighten's history")?;
        let (s, d) = (PathBuf::from(&rec.src), PathBuf::from(&rec.dest));
        if !d.is_dir() { return Err("The moved copy is missing, cannot restore".into()); }
        if !fs::symlink_metadata(&s).map(|m| is_reparse(&m)).unwrap_or(false) { return Err("Original path is no longer a link".into()); }
        if !busy_in(&s, &running_procs()).is_empty() { return Err("The app is running. Close it first".into()); }
        let before = dir_size(&d, 0);
        let c_free = drives().iter().find(|x| lc(Path::new(&x.letter)) == lc(&PathBuf::from(&rec.src.chars().take(2).collect::<String>()))).map(|x| x.free).unwrap_or(0);
        if before.bytes + before.bytes / 20 > c_free { return Err("Not enough free space to move back".into()); }
        step(&app, "unlink", "Removing the link");
        fs::remove_dir(&s).map_err(|e| format!("Could not remove link: {e}"))?; // removes the link only
        let relink = |msg: String| -> String { let _ = fs::remove_dir_all(&s); let _ = mk_junction(&s, &d); msg };
        step(&app, "copy", "Copying back");
        fs::create_dir_all(&s).map_err(|e| relink(e.to_string()))?;
        robocopy(&d, &s).map_err(&relink)?;
        step(&app, "verify", "Checking file count and size match");
        if !same(before, dir_size(&s, 0)) { return Err(relink("Copy check failed, link restored".into())); }
        step(&app, "clean", "Removing the copy on the other drive");
        let _ = fs::remove_dir_all(&d);
        j.retain(|m| lc(Path::new(&m.src)) != lc(&s));
        save_journal(&j)
    })
    .await?
}

// ---------- admin tasks (fixed scripts; the app itself runs elevated) ----------
#[tauri::command]
async fn admin_task(task: String) -> Result<String, String> {
    blocking(move || -> Result<String, String> {
        let dir = envp("LOCALAPPDATA").join("Lighten");
        let _ = fs::create_dir_all(&dir);
        let log = dir.join("admin.log");
        let script = match task.as_str() {
            "dism" => "Dism.exe /Online /Cleanup-Image /StartComponentCleanup".to_string(),
            "update_cache" => "$ErrorActionPreference='Continue'; Stop-Service wuauserv,bits,dosvc -Force -ErrorAction SilentlyContinue; \
                foreach($p in @(\"$env:SystemRoot\\SoftwareDistribution\\Download\",\"$env:ProgramData\\Microsoft\\Windows\\DeliveryOptimization\\Cache\")){ if(Test-Path $p){ Get-ChildItem $p -Force | Remove-Item -Recurse -Force -ErrorAction SilentlyContinue } }; \
                Start-Service wuauserv,bits,dosvc -ErrorAction SilentlyContinue".to_string(),
            "wintemp" => "Get-ChildItem \"$env:SystemRoot\\Temp\" -Force | Where-Object { $_.LastWriteTime -lt (Get-Date).AddHours(-1) -and -not ($_.Attributes -band 'ReparsePoint') } | Remove-Item -Recurse -Force -ErrorAction SilentlyContinue".to_string(),
            _ => return Err("Unknown task".into()),
        };
        let ps1 = dir.join(format!("admin-{task}.ps1"));
        fs::write(&ps1, format!("& {{ {script} }} *> '{}'\r\n'DONE' | Add-Content '{}'\r\n", log.display(), log.display())).map_err(|e| e.to_string())?;
        let _ = fs::remove_file(&log);
        let o = run("powershell", &["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", &ps1.to_string_lossy()])?;
        if !o.status.success() { return Err("The task failed to run".into()); }
        Ok(fs::read_to_string(&log).unwrap_or_default().trim().chars().rev().take(600).collect::<String>().chars().rev().collect())
    })
    .await?
}

// ---------- existing links ----------
#[derive(Serialize)]
struct Found {
    src: String,
    dest: String,
    bytes: u64,
    tracked: bool,
}
fn scan_links(dir: &Path, depth: u32, out: &mut Vec<(PathBuf, PathBuf)>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let Ok(md) = fs::symlink_metadata(&p) else { continue };
        if is_reparse(&md) {
            if let Ok(t) = fs::read_link(&p) {
                let t = PathBuf::from(t.to_string_lossy().trim_start_matches(r"\\?\"));
                let other_drive = lc(&p).chars().next() != lc(&t).chars().next();
                if other_drive && t.is_dir() {
                    out.push((p, t));
                }
            }
        } else if md.is_dir() && depth > 0 {
            scan_links(&p, depth - 1, out);
        }
    }
}
#[tauri::command]
async fn find_links() -> Result<Vec<Found>, String> {
    blocking(|| {
        let prof = envp("USERPROFILE");
        let mut raw = vec![];
        for (d, depth) in [
            (prof.clone(), 0), (envp("LOCALAPPDATA"), 0), (envp("APPDATA"), 0),
            (envp("ProgramFiles"), 1), (envp("ProgramFiles(x86)"), 1), (envp("ProgramData"), 1),
        ] {
            scan_links(&d, depth, &mut raw);
        }
        let j = load_journal();
        raw.into_iter()
            .map(|(src, dest)| Found {
                bytes: dir_size(&dest, 0).bytes,
                tracked: j.iter().any(|m| lc(Path::new(&m.src)) == lc(&src)),
                src: src.to_string_lossy().into(),
                dest: dest.to_string_lossy().into(),
            })
            .collect()
    })
    .await
}
#[tauri::command]
async fn adopt_link(src: String) -> Result<(), String> {
    blocking(move || -> Result<(), String> {
        let mut raw = vec![];
        let sp = PathBuf::from(&src);
        scan_links(sp.parent().ok_or("Bad path")?, 0, &mut raw);
        let (s, d) = raw.into_iter().find(|(s, _)| lc(s) == lc(&sp)).ok_or("Not a link to another drive")?;
        let mut j = load_journal();
        if j.iter().any(|m| lc(Path::new(&m.src)) == lc(&s)) {
            return Ok(());
        }
        j.push(MoveRec { src: s.to_string_lossy().into(), dest: d.to_string_lossy().into(), bytes: dir_size(&d, 0).bytes, when: now() });
        save_journal(&j)
    })
    .await?
}

// ---------- settings and schedule ----------
#[derive(Serialize, Deserialize, Clone, Default)]
struct Settings {
    #[serde(default)]
    auto_scan: bool,
    #[serde(default)]
    every_days: u32,
    #[serde(default)]
    auto_clean: bool,
    #[serde(default)]
    update_url: String,
    #[serde(default)]
    tray_on_close: bool,
}
fn lighten_dir() -> PathBuf {
    let d = envp("LOCALAPPDATA").join("Lighten");
    let _ = fs::create_dir_all(&d);
    d
}
fn load_settings() -> Settings {
    fs::read_to_string(lighten_dir().join("settings.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}
const TASK: &str = "Lighten scan";
fn apply_schedule(s: &Settings) -> Result<(), String> {
    apply_schedule_for(s, &std::env::current_exe().map_err(|e| e.to_string())?)
}
fn apply_schedule_for(s: &Settings, exe: &Path) -> Result<(), String> {
    if s.auto_scan {
        let tr = format!("\"{}\" --scan", exe.display());
        let days = s.every_days.clamp(1, 30).to_string();
        let o = run("schtasks", &["/Create", "/TN", TASK, "/TR", &tr, "/SC", "DAILY", "/MO", &days, "/ST", "12:00", "/RL", "HIGHEST", "/F"])?;
        if !o.status.success() {
            return Err(format!("Could not create the scheduled task: {}", String::from_utf8_lossy(&o.stderr)));
        }
    } else {
        let _ = run("schtasks", &["/Delete", "/TN", TASK, "/F"]);
    }
    Ok(())
}
#[tauri::command]
fn get_settings() -> Settings {
    load_settings()
}
#[tauri::command]
fn save_settings(mut s: Settings) -> Result<Settings, String> {
    s.every_days = s.every_days.clamp(1, 30);
    s.update_url = s.update_url.trim().to_string();
    apply_schedule(&s)?;
    fs::write(lighten_dir().join("settings.json"), serde_json::to_string_pretty(&s).unwrap()).map_err(|e| e.to_string())?;
    Ok(s)
}
#[tauri::command]
fn last_scan() -> Option<serde_json::Value> {
    fs::read_to_string(lighten_dir().join("last-scan.json")).ok().and_then(|s| serde_json::from_str(&s).ok())
}
/// Runs from Task Scheduler: scan, optionally clean the default-safe set, record the result.
fn headless() {
    let st = load_settings();
    let cats = scan_cats("C:");
    let ok = |c: &&CatInfo| c.busy.is_none() && c.id != "recycle" && c.bytes > 0;
    let cleanable: u64 = cats.iter().filter(ok).map(|c| c.bytes).sum();
    let mut freed = 0;
    if st.auto_clean {
        let ids = cats.iter().filter(ok).filter(|c| c.default_on).map(|c| c.id.clone()).collect();
        freed = do_clean(ids, "C:").iter().map(|r| r.freed).sum::<u64>();
    }
    let c_free = drives().iter().find(|d| d.letter.eq_ignore_ascii_case("C:")).map(|d| d.free).unwrap_or(0);
    let v = serde_json::json!({ "when": now(), "cleanable": cleanable, "freed": freed, "auto_clean": st.auto_clean, "c_free": c_free });
    let _ = fs::write(lighten_dir().join("last-scan.json"), v.to_string());
    tools::record_snapshot(false);
    if cleanable > 500 * 1024 * 1024 || freed > 0 {
        let msg = if freed > 0 { format!("Cleaned {:.1} GB. C: has {:.0} GB free.", freed as f64 / 1073741824.0, c_free as f64 / 1073741824.0) } else { format!("{:.1} GB can be cleaned. Open Lighten to review.", cleanable as f64 / 1073741824.0) };
        tools::toast("Lighten scan finished", &msg);
    }
}

// ---------- delta updater ----------
use sha2::{Digest, Sha256};
use std::io::Read;
#[derive(Deserialize)]
struct Pkg {
    url: String,
    sha256: String,
}
#[derive(Deserialize)]
struct Manifest {
    version: String,
    #[serde(default)]
    signature: String,
    full: Pkg,
    #[serde(default)]
    patches: std::collections::HashMap<String, Pkg>, // key: version being updated from
}
fn sha_hex(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}
fn ver(s: &str) -> Vec<u32> {
    s.trim_start_matches('v').split('.').map(|p| p.parse().unwrap_or(0)).collect()
}
/// Remote sources must be https; a plain path is allowed for offline/testing.
fn fetch(src: &str) -> Result<Vec<u8>, String> {
    if src.starts_with("https://") {
        let r = ureq::get(src).timeout(std::time::Duration::from_secs(60)).call().map_err(|e| format!("Download failed: {e}"))?;
        let mut buf = vec![];
        r.into_reader().take(400 * 1024 * 1024).read_to_end(&mut buf).map_err(|e| e.to_string())?;
        Ok(buf)
    } else if src.starts_with("http://") {
        Err("Updates must use https".into())
    } else {
        fs::read(src).map_err(|e| format!("Cannot read {src}: {e}"))
    }
}
fn manifest() -> Result<Manifest, String> {
    let url = load_settings().update_url;
    if url.is_empty() {
        return Err("Set an update URL in Settings first".into());
    }
    let m: Manifest = serde_json::from_slice(&fetch(&url)?).map_err(|e| format!("Bad update manifest: {e}"))?;
    tools::verify_signature(&m.version, &m.full.sha256, &m.signature)?;
    Ok(m)
}
#[derive(Serialize)]
struct UpdInfo {
    current: String,
    latest: String,
    available: bool,
    delta: bool,
}
#[tauri::command]
async fn app_info() -> serde_json::Value {
    serde_json::json!({ "version": env!("CARGO_PKG_VERSION"), "exe": std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_default() })
}
#[tauri::command]
async fn check_update() -> Result<UpdInfo, String> {
    blocking(|| -> Result<UpdInfo, String> {
        let m = manifest()?;
        let cur = env!("CARGO_PKG_VERSION");
        Ok(UpdInfo { current: cur.into(), latest: m.version.clone(), available: ver(&m.version) > ver(cur), delta: m.patches.contains_key(cur) })
    })
    .await?
}
fn build_update(m: &Manifest, exe: &Path) -> Result<Vec<u8>, String> {
    let cur = env!("CARGO_PKG_VERSION");
    if let Some(pt) = m.patches.get(cur) {
        let attempt = (|| -> Result<Vec<u8>, String> {
            let patch = fetch(&pt.url)?;
            if sha_hex(&patch) != pt.sha256.to_lowercase() { return Err("patch hash mismatch".into()); }
            let old = fs::read(exe).map_err(|e| e.to_string())?;
            let mut out = Vec::new();
            qbsdiff::Bspatch::new(&patch).map_err(|e| e.to_string())?.apply(&old, &mut out).map_err(|e| e.to_string())?;
            if sha_hex(&out) != m.full.sha256.to_lowercase() { return Err("patched file hash mismatch".into()); }
            Ok(out)
        })();
        if let Ok(b) = attempt { return Ok(b); } // otherwise fall back to the full download
    }
    let b = fetch(&m.full.url)?;
    if sha_hex(&b) != m.full.sha256.to_lowercase() {
        return Err("Downloaded file failed its hash check; not installed".into());
    }
    Ok(b)
}
#[tauri::command]
async fn apply_update(app: AppHandle) -> Result<(), String> {
    blocking(move || -> Result<(), String> {
        let m = manifest()?;
        if ver(&m.version) <= ver(env!("CARGO_PKG_VERSION")) { return Err("Already up to date".into()); }
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let bytes = build_update(&m, &exe)?;
        if bytes.len() < 1_000_000 || &bytes[..2] != b"MZ" { return Err("Update file is not a valid program".into()); }
        let new = exe.with_extension("update.exe");
        let bak = exe.with_extension("bak");
        fs::write(&new, &bytes).map_err(|e| e.to_string())?;
        let q = |p: &Path| p.to_string_lossy().replace('\'', "''");
        let script = format!(
            "Copy-Item -LiteralPath '{e}' -Destination '{b}' -Force; for($i=0;$i -lt 30;$i++){{ try{{ Move-Item -LiteralPath '{n}' -Destination '{e}' -Force -ErrorAction Stop; break }}catch{{ Start-Sleep 1 }} }}; Start-Process -FilePath '{e}'",
            e = q(&exe), b = q(&bak), n = q(&new)
        );
        Command::new("powershell").args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &script]).creation_flags(NO_WINDOW | 0x8).spawn().map_err(|e| e.to_string())?;
        app.exit(0);
        Ok(())
    })
    .await?
}
/// `Lighten.exe --make-patch old.exe new.exe out.patch` writes the patch and a hashes file for the manifest.
fn make_patch(a: &[String]) {
    let res = (|| -> Result<(), String> {
        if a.len() < 3 { return Err("usage: --make-patch old new out".into()); }
        let (old, new) = (fs::read(&a[0]).map_err(|e| e.to_string())?, fs::read(&a[1]).map_err(|e| e.to_string())?);
        let mut patch = Vec::new();
        qbsdiff::Bsdiff::new(&old, &new).compare(&mut patch).map_err(|e| e.to_string())?;
        fs::write(&a[2], &patch).map_err(|e| e.to_string())?;
        let j = serde_json::json!({ "new_sha256": sha_hex(&new), "patch_sha256": sha_hex(&patch), "new_bytes": new.len(), "patch_bytes": patch.len() });
        fs::write(format!("{}.json", a[2]), serde_json::to_string_pretty(&j).unwrap()).map_err(|e| e.to_string())
    })();
    if let Err(e) = res { let _ = fs::write("make-patch-error.txt", e); }
}

#[tauri::command]
fn open_in_explorer(path: String) {
    let _ = Command::new("explorer").arg(path).spawn();
}

/// `Lighten.exe --apply-patch old new-out patch`: offline apply, used to verify patches before publishing.
fn apply_patch_cli(a: &[String]) {
    let res = (|| -> Result<(), String> {
        if a.len() < 3 { return Err("usage: --apply-patch old out patch".into()); }
        let (old, patch) = (fs::read(&a[0]).map_err(|e| e.to_string())?, fs::read(&a[2]).map_err(|e| e.to_string())?);
        let mut out = Vec::new();
        qbsdiff::Bspatch::new(&patch).map_err(|e| e.to_string())?.apply(&old, &mut out).map_err(|e| e.to_string())?;
        fs::write(&a[1], &out).map_err(|e| e.to_string())
    })();
    if let Err(e) = res { let _ = fs::write("apply-patch-error.txt", e); }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let arg_after = |f: &str| a.iter().position(|x| x == f).map(|i| a[i + 1..].to_vec());
    if a.iter().any(|x| x == "--scan") { headless(); return; }
    if let Some(r) = arg_after("--make-patch") { make_patch(&r); return; }
    if let Some(r) = arg_after("--apply-patch") { apply_patch_cli(&r); return; }
    if let Some(r) = arg_after("--gen-key") { tools::gen_key(&r); return; }
    if let Some(r) = arg_after("--sign") { tools::sign_cli(&r); return; }
    if a.iter().any(|x| x == "--uninstall") { tools::do_uninstall(); return; }
    tauri::Builder::default()
        .setup(|app| {
            use tauri::menu::{Menu, MenuItem};
            use tauri::tray::TrayIconBuilder;
            use tauri::Manager;
            let show = MenuItem::with_id(app, "show", "Open Lighten", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let mut tb = TrayIconBuilder::new().menu(&menu).tooltip("Lighten").on_menu_event(|app, e| match e.id.as_ref() {
                "show" => { if let Some(w) = app.get_webview_window("main") { let _ = w.show(); let _ = w.set_focus(); } }
                "quit" => app.exit(0),
                _ => {}
            });
            if let Some(ic) = app.default_window_icon() { tb = tb.icon(ic.clone()); }
            tb.build(app)?;
            Ok(())
        })
        .on_window_event(|w, e| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = e {
                if load_settings().tray_on_close { api.prevent_close(); let _ = w.hide(); }
            }
        })
        .invoke_handler(tauri::generate_handler![drives, scan_clean, clean, list_dir, move_candidates, plan_move, move_folder, moved_list, move_back, admin_task, open_in_explorer, find_links, adopt_link, get_settings, save_settings, last_scan, app_info, check_update, apply_update,
            tools::find_large, tools::find_dupes, tools::find_installers, tools::trash_files, tools::find_dev, tools::delete_dev, tools::reveal_file, tools::system_info, tools::system_task, tools::apps_list, tools::uninstall_app, tools::snapshot, tools::growth, tools::export_report, tools::install_state, tools::install_app, tools::uninstall_self])
        .run(tauri::generate_context!())
        .expect("failed to start Lighten");
}
