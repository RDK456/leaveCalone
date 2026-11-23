//! Finders, apps, system tasks, growth history, update signing, installer. Shares helpers with main.rs.
use super::*;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;
use std::os::windows::process::CommandExt as _;
use std::sync::Mutex;

// ---------- offered-path allowlist: deletes only act on paths a finder just showed ----------
static OFFERED: Mutex<Option<HashSet<String>>> = Mutex::new(None);
pub(crate) fn offer(paths: impl IntoIterator<Item = String>) {
    let mut g = OFFERED.lock().unwrap();
    let s = g.get_or_insert_with(HashSet::new);
    for p in paths {
        s.insert(lc(Path::new(&p)));
    }
}
pub(crate) fn offered(p: &str) -> bool {
    OFFERED.lock().unwrap().as_ref().map(|s| s.contains(&lc(Path::new(p)))).unwrap_or(false)
}
pub(crate) fn protected(p: &Path) -> bool {
    let l = lc(p);
    [envp("SystemRoot"), envp("ProgramFiles"), envp("ProgramFiles(x86)"), envp("ProgramData")]
        .iter()
        .any(|b| !lc(b).is_empty() && (l == lc(b) || l.starts_with(&(lc(b) + "\\"))))
        || super::extra::excluded(p)
        || l.contains("\\$recycle.bin")
        || l.contains("\\system volume information")
}
pub(crate) fn mtime(md: &fs::Metadata) -> u64 {
    md.modified().ok().and_then(|m| m.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}
pub(crate) fn run_ps(script: &str) -> Result<String, String> {
    let dir = lighten_dir();
    let f = dir.join(format!("run-{}.ps1", now()));
    fs::write(&f, format!("[Console]::OutputEncoding=[Text.Encoding]::UTF8\r\n{script}\r\n")).map_err(|e| e.to_string())?;
    let o = Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&f)
        .creation_flags(NO_WINDOW)
        .output()
        .map_err(|e| e.to_string());
    let _ = fs::remove_file(&f);
    let o = o?;
    let mut s = String::from_utf8_lossy(&o.stdout).to_string();
    s.push_str(&String::from_utf8_lossy(&o.stderr));
    Ok(s.trim().to_string())
}
pub(crate) fn tail(s: &str, n: usize) -> String {
    let c: Vec<char> = s.chars().collect();
    c[c.len().saturating_sub(n)..].iter().collect()
}

// ---------- file walking ----------
const SKIP: &[&str] = &["windows", "$recycle.bin", "system volume information", "program files", "program files (x86)", "programdata", "recovery", "perflogs", "windowsapps", "msocache", ".git", "node_modules"];
pub(crate) fn walk_files(root: &Path, appdata: bool, f: &mut dyn FnMut(&Path, &fs::Metadata)) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(md) = fs::symlink_metadata(&p) else { continue };
            if is_reparse(&md) || super::extra::excluded(&p) {
                continue;
            }
            if md.is_dir() {
                let n = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
                if SKIP.contains(&n.as_str()) || (!appdata && n == "appdata") {
                    continue;
                }
                stack.push(p);
            } else {
                f(&p, &md);
            }
        }
    }
}
pub(crate) fn root_ok(root: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(root.trim());
    if !p.is_absolute() || !p.is_dir() {
        return Err("Pick an existing folder".into());
    }
    Ok(p)
}

#[derive(Serialize, Clone)]
pub struct FileHit {
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) bytes: u64,
    pub(crate) modified: u64,
}
pub(crate) fn hit(p: &Path, md: &fs::Metadata) -> FileHit {
    FileHit { path: p.to_string_lossy().into(), name: p.file_name().map(|n| n.to_string_lossy().into()).unwrap_or_default(), bytes: md.len(), modified: mtime(md) }
}

#[tauri::command]
pub async fn find_large(root: String, min_mb: u64) -> Result<Vec<FileHit>, String> {
    blocking(move || -> Result<Vec<FileHit>, String> {
        let root = root_ok(&root)?;
        let min = min_mb.max(1) * 1024 * 1024;
        // NTFS master-file-table read when possible (seconds instead of minutes); otherwise walk.
        let mut v = super::extra::mft_big(&root, min).unwrap_or_else(|| {
            let mut v = vec![];
            walk_files(&root, true, &mut |p, md| {
                if md.len() >= min {
                    v.push(hit(p, md));
                }
            });
            v
        });
        v.sort_by(|a, b| b.bytes.cmp(&a.bytes));
        v.truncate(200);
        offer(v.iter().map(|h| h.path.clone()));
        Ok(v)
    })
    .await?
}

fn hash_file(p: &Path, limit: Option<usize>) -> Option<String> {
    let mut f = fs::File::open(p).ok()?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut left = limit.unwrap_or(usize::MAX);
    while left > 0 {
        let want = buf.len().min(left);
        let n = f.read(&mut buf[..want]).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        left -= n;
    }
    Some(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}
#[derive(Serialize)]
pub struct DupGroup {
    pub(crate) bytes: u64,
    pub(crate) files: Vec<FileHit>,
}
#[tauri::command]
pub async fn find_dupes(root: String, min_mb: u64) -> Result<Vec<DupGroup>, String> {
    blocking(move || -> Result<Vec<DupGroup>, String> {
        let root = root_ok(&root)?;
        let min = min_mb.max(1) * 1024 * 1024;
        let mut by_size: HashMap<u64, Vec<FileHit>> = HashMap::new();
        walk_files(&root, false, &mut |p, md| {
            if md.len() >= min {
                by_size.entry(md.len()).or_default().push(hit(p, md));
            }
        });
        let mut out = vec![];
        for (size, files) in by_size.into_iter().filter(|(_, v)| v.len() > 1) {
            let mut partial: HashMap<String, Vec<FileHit>> = HashMap::new();
            for f in files {
                if let Some(h) = hash_file(Path::new(&f.path), Some(65536)) {
                    partial.entry(h).or_default().push(f);
                }
            }
            for (_, grp) in partial.into_iter().filter(|(_, v)| v.len() > 1) {
                let mut full: HashMap<String, Vec<FileHit>> = HashMap::new();
                for f in grp {
                    if let Some(h) = hash_file(Path::new(&f.path), None) {
                        full.entry(h).or_default().push(f);
                    }
                }
                for (_, mut g) in full.into_iter().filter(|(_, v)| v.len() > 1) {
                    g.sort_by_key(|f| f.modified); // oldest first: the one to keep
                    out.push(DupGroup { bytes: size, files: g });
                }
            }
        }
        out.sort_by(|a, b| (b.bytes * (b.files.len() as u64 - 1)).cmp(&(a.bytes * (a.files.len() as u64 - 1))));
        out.truncate(100);
        offer(out.iter().flat_map(|g| g.files.iter().map(|f| f.path.clone())));
        Ok(out)
    })
    .await?
}

#[tauri::command]
pub async fn find_installers(days: u64) -> Result<Vec<FileHit>, String> {
    blocking(move || {
        let prof = envp("USERPROFILE");
        let cutoff = now().saturating_sub(days * 86400);
        let all = ["iso", "msi", "exe", "msix", "appx", "msixbundle", "appxbundle", "cab"];
        let safe = ["iso", "msi", "msix", "appx", "msixbundle", "appxbundle"];
        let mut v = vec![];
        for (dir, exts) in [("Downloads", &all[..]), ("Desktop", &safe[..]), ("Documents", &safe[..])] {
            walk_files(&prof.join(dir), false, &mut |p, md| {
                let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                if exts.contains(&ext.as_str()) && mtime(md) <= cutoff {
                    v.push(hit(p, md));
                }
            });
        }
        v.sort_by(|a, b| b.bytes.cmp(&a.bytes));
        offer(v.iter().map(|h| h.path.clone()));
        v
    })
    .await
}

#[derive(Serialize)]
pub struct TrashResult {
    freed: u64,
    done: u64,
    failed: u64,
    recycled: bool,
}
/// Files go to the Recycle Bin unless `permanent`. Only paths a finder offered, never protected locations.
#[tauri::command]
pub async fn trash_files(paths: Vec<String>, permanent: bool) -> Result<TrashResult, String> {
    blocking(move || -> Result<TrashResult, String> {
        let mut ok: Vec<(PathBuf, u64)> = vec![];
        let mut failed = 0;
        for s in &paths {
            let p = PathBuf::from(s);
            match fs::symlink_metadata(&p) {
                Ok(md) if offered(s) && md.is_file() && !is_reparse(&md) && !protected(&p) => ok.push((p, md.len())),
                _ => failed += 1,
            }
        }
        let (mut freed, mut done) = (0, 0);
        if permanent {
            for (p, len) in &ok {
                if fs::remove_file(p).is_ok() {
                    freed += len;
                    done += 1;
                } else {
                    failed += 1;
                }
            }
        } else if !ok.is_empty() {
            let list = lighten_dir().join("trash-list.txt");
            fs::write(&list, ok.iter().map(|(p, _)| p.to_string_lossy().to_string()).collect::<Vec<_>>().join("\r\n")).map_err(|e| e.to_string())?;
            let script = format!(
                "Add-Type -AssemblyName Microsoft.VisualBasic; Get-Content -LiteralPath '{}' -Encoding UTF8 | ForEach-Object {{ try {{ [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteFile($_,'OnlyErrorDialogs','SendToRecycleBin'); 'ok' }} catch {{ 'fail' }} }}",
                list.display()
            );
            let out = run_ps(&script)?;
            let _ = fs::remove_file(&list);
            let mut logged = vec![];
            for ((p, len), line) in ok.iter().zip(out.lines().map(|l| l.trim())) {
                if line == "ok" && !p.exists() {
                    freed += len;
                    done += 1;
                    logged.push((p.to_string_lossy().to_string(), *len));
                } else {
                    failed += 1;
                }
            }
            super::extra::log_trash(&logged); // feeds the Undo list
        }
        Ok(TrashResult { freed, done, failed, recycled: !permanent })
    })
    .await?
}

// ---------- dev leftovers ----------
#[derive(Serialize)]
pub struct DevHit {
    path: String,
    kind: String,
    project: String,
    bytes: u64,
    idle_days: u64,
}
#[tauri::command]
pub async fn find_dev(root: String, days: u64) -> Result<Vec<DevHit>, String> {
    blocking(move || -> Result<Vec<DevHit>, String> {
        let root = root_ok(&root)?;
        let mut found: Vec<(PathBuf, &'static str, PathBuf)> = vec![];
        let mut stack = vec![(root, 0u32)];
        while let Some((d, depth)) = stack.pop() {
            let Ok(rd) = fs::read_dir(&d) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                let Ok(md) = fs::symlink_metadata(&p) else { continue };
                if !md.is_dir() || is_reparse(&md) || super::extra::excluded(&p) {
                    continue;
                }
                let n = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
                let kind = if n == "node_modules" && d.join("package.json").exists() {
                    Some("node_modules")
                } else if n == "target" && d.join("Cargo.toml").exists() {
                    Some("Rust target")
                } else if (n == ".venv" || n == "venv") && p.join("pyvenv.cfg").exists() {
                    Some("Python venv")
                } else {
                    None
                };
                if let Some(k) = kind {
                    found.push((p, k, d.clone()));
                } else if depth < 8 && !SKIP.contains(&n.as_str()) && n != "appdata" && n != "target" {
                    stack.push((p, depth + 1));
                }
            }
        }
        let paths: Vec<PathBuf> = found.iter().map(|f| f.0.clone()).collect();
        let sizes = sizes_parallel(&paths, 0);
        let n = now();
        let mut out = vec![];
        for ((p, kind, proj), sz) in found.into_iter().zip(sizes) {
            let newest = fs::read_dir(&proj)
                .map(|rd| rd.flatten().filter(|e| e.path() != p).filter_map(|e| fs::symlink_metadata(e.path()).ok()).map(|m| mtime(&m)).max().unwrap_or(0))
                .unwrap_or(0);
            let idle = n.saturating_sub(newest) / 86400;
            if idle >= days && sz.bytes > 0 {
                out.push(DevHit { path: p.to_string_lossy().into(), kind: kind.into(), project: proj.to_string_lossy().into(), bytes: sz.bytes, idle_days: idle });
            }
        }
        out.sort_by(|a, b| b.bytes.cmp(&a.bytes));
        offer(out.iter().map(|h| h.path.clone()));
        Ok(out)
    })
    .await?
}
#[derive(Serialize)]
pub struct DelResult {
    freed: u64,
    done: u64,
    failed: u64,
}
#[tauri::command]
pub async fn delete_dev(paths: Vec<String>) -> Result<DelResult, String> {
    blocking(move || {
        let (mut freed, mut done, mut failed) = (0, 0, 0);
        for s in paths {
            let p = PathBuf::from(&s);
            let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            let okname = ["node_modules", "target", ".venv", "venv"].contains(&name.as_str());
            let Ok(md) = fs::symlink_metadata(&p) else { failed += 1; continue };
            if !offered(&s) || !okname || !md.is_dir() || is_reparse(&md) || protected(&p) {
                failed += 1;
                continue;
            }
            let size = dir_size(&p, 0).bytes;
            if fs::remove_dir_all(&p).is_err() {
                let _ = Command::new("cmd").args(["/C", "rmdir", "/s", "/q"]).arg(&p).creation_flags(NO_WINDOW).output();
            }
            if p.exists() { failed += 1 } else { freed += size; done += 1 }
        }
        DelResult { freed, done, failed }
    })
    .await
}
#[tauri::command]
pub fn reveal_file(path: String) {
    let _ = Command::new("explorer").raw_arg(format!("/select,\"{path}\"")).spawn();
}

// ---------- system ----------
pub(crate) fn find_ext(root: &Path, ext: &str, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(root) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let Ok(md) = fs::symlink_metadata(&p) else { continue };
        if md.is_dir() && !is_reparse(&md) && depth > 0 {
            find_ext(&p, ext, depth - 1, out);
        } else if md.is_file() && p.extension().map(|x| x.to_string_lossy().eq_ignore_ascii_case(ext)).unwrap_or(false) {
            out.push(p);
        }
    }
}
fn find_vhdx() -> Vec<PathBuf> {
    let l = envp("LOCALAPPDATA");
    let mut v = vec![];
    for r in [l.join("Packages"), l.join("Docker"), l.join("wsl")] {
        find_ext(&r, "vhdx", 4, &mut v);
    }
    v
}
#[derive(Serialize)]
pub struct Vhdx {
    path: String,
    bytes: u64,
}
#[derive(Serialize)]
pub struct SysInfo {
    windows_old: u64,
    hiber: u64,
    pagefile: Vec<String>,
    docker: Option<Vec<String>>,
    vhdx: Vec<Vhdx>,
}
#[tauri::command]
pub async fn system_info() -> Result<SysInfo, String> {
    blocking(|| {
        let wo = Path::new(r"C:\Windows.old");
        let windows_old = if wo.is_dir() { dir_size(wo, 0).bytes } else { 0 };
        let hiber = fs::metadata(r"C:\hiberfil.sys").map(|m| m.len()).unwrap_or(0);
        let pagefile = run_ps("Get-CimInstance Win32_PageFileUsage | ForEach-Object { \"$($_.Name)  allocated $($_.AllocatedBaseSize) MB, peak $($_.PeakUsage) MB\" }").unwrap_or_default().lines().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        let docker = Command::new("docker").args(["system", "df", "--format", "{{.Type}}: {{.Size}} ({{.Reclaimable}} reclaimable)"]).creation_flags(NO_WINDOW).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).lines().map(|s| s.to_string()).collect());
        let vhdx = find_vhdx().into_iter().map(|p| Vhdx { bytes: fs::metadata(&p).map(|m| m.len()).unwrap_or(0), path: p.to_string_lossy().into() }).collect();
        SysInfo { windows_old, hiber, pagefile, docker, vhdx }
    })
    .await
}
#[tauri::command]
pub async fn system_task(task: String, arg: Option<String>) -> Result<String, String> {
    blocking(move || -> Result<String, String> {
        let script = match task.as_str() {
            "winold" => r#"$p='C:\Windows.old'; if(-not (Test-Path $p)){ 'Nothing to remove'; exit }
takeown /F $p /R /A /D Y | Out-Null; icacls $p /grant Administrators:F /T /C /Q | Out-Null
cmd /c rd /s /q $p; if(Test-Path $p){ 'Partly removed. Some files were in use.' } else { 'Removed' }"#.to_string(),
            "hiber_off" => "powercfg /h off; 'Hibernation off (Fast Startup is off too)'".into(),
            "hiber_on" => "powercfg /h on; 'Hibernation on'".into(),
            "docker_prune" => "docker system prune -f".into(),
            "docker_prune_all" => "docker system prune -a -f".into(),
            "restore_point" => "Enable-ComputerRestore -Drive 'C:\\' -ErrorAction SilentlyContinue; try { Checkpoint-Computer -Description 'Lighten' -RestorePointType MODIFY_SETTINGS -ErrorAction Stop; 'Restore point created' } catch { 'Not created: ' + $_.Exception.Message }".into(),
            "wsl_compact" => {
                let a = arg.ok_or("Missing disk")?;
                if !find_vhdx().iter().any(|p| lc(p) == lc(Path::new(&a))) { return Err("Not a known WSL or Docker disk".into()); }
                format!("wsl --shutdown; Start-Sleep 3; $t=Join-Path $env:TEMP 'lighten-compact.txt'; \"select vdisk file=`\"{a}`\"`r`nattach vdisk readonly`r`ncompact vdisk`r`ndetach vdisk\" | Set-Content $t -Encoding ASCII; diskpart /s $t; Remove-Item $t")
            }
            _ => return Err("Unknown task".into()),
        };
        Ok(tail(&run_ps(&script)?, 800))
    })
    .await?
}

// ---------- installed apps ----------
struct RawApp {
    id: String,
    name: String,
    version: String,
    publisher: String,
    location: String,
    est: u64,
    installed: String,
    uninstall: String,
}
fn read_apps() -> Vec<RawApp> {
    let script = r#"$k='HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*','HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*','HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*'
@(Get-ItemProperty $k -ErrorAction SilentlyContinue | Where-Object { $_.DisplayName -and $_.UninstallString -and -not $_.SystemComponent } | Select-Object PSChildName,DisplayName,DisplayVersion,Publisher,InstallLocation,EstimatedSize,InstallDate,UninstallString) | ConvertTo-Json -Compress"#;
    let Ok(out) = run_ps(script) else { return vec![] };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&out) else { return vec![] };
    let arr = match v { serde_json::Value::Array(a) => a, o @ serde_json::Value::Object(_) => vec![o], _ => vec![] };
    let s = |x: &serde_json::Value, k: &str| x.get(k).and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let mut seen = HashSet::new();
    arr.iter()
        .map(|x| RawApp { id: format!("{}|{}", s(x, "PSChildName"), s(x, "DisplayName")), name: s(x, "DisplayName"), version: s(x, "DisplayVersion"), publisher: s(x, "Publisher"), location: s(x, "InstallLocation").trim_matches('"').trim_end_matches('\\').to_string(), est: x.get("EstimatedSize").and_then(|v| v.as_u64()).unwrap_or(0) * 1024, installed: s(x, "InstallDate"), uninstall: s(x, "UninstallString") })
        .filter(|a| seen.insert(a.id.clone()))
        .collect()
}
#[derive(Serialize)]
pub struct AppRow {
    id: String,
    name: String,
    version: String,
    publisher: String,
    location: String,
    bytes: u64,
    installed: String,
    last_used: Option<u64>,
    movable: bool,
}
#[tauri::command]
pub async fn apps_list() -> Result<Vec<AppRow>, String> {
    blocking(|| {
        let apps = read_apps();
        let mut pre: HashMap<String, u64> = HashMap::new();
        if let Ok(rd) = fs::read_dir(r"C:\Windows\Prefetch") {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                if let Some(stem) = n.strip_suffix(".pf") {
                    if let Some((exe, _)) = stem.rsplit_once('-') {
                        let t = e.metadata().map(|m| mtime(&m)).unwrap_or(0);
                        let k = exe.to_lowercase();
                        let cur = pre.entry(k).or_insert(0);
                        *cur = (*cur).max(t);
                    }
                }
            }
        }
        let locs: Vec<PathBuf> = apps.iter().map(|a| PathBuf::from(&a.location)).collect();
        let valid: Vec<bool> = locs.iter().map(|p| p.is_dir() && p.components().count() > 2).collect();
        let to_size: Vec<PathBuf> = locs.iter().zip(&valid).filter(|(_, v)| **v).map(|(p, _)| p.clone()).collect();
        let mut sizes = sizes_parallel(&to_size, 0).into_iter();
        let prof = envp("USERPROFILE");
        let pfs = [envp("ProgramFiles"), envp("ProgramFiles(x86)")];
        apps.into_iter().zip(locs).zip(valid).map(|((a, loc), ok)| {
            let bytes = if ok { sizes.next().map(|s| s.bytes).unwrap_or(0) } else { a.est };
            let mut last = None;
            if ok {
                let mut exes = vec![];
                find_ext(&loc, "exe", 2, &mut exes);
                for e in exes {
                    if let Some(t) = e.file_name().and_then(|n| pre.get(&n.to_string_lossy().to_lowercase())) {
                        last = Some(last.map_or(*t, |l: u64| l.max(*t)));
                    }
                }
            }
            let under_pf = pfs.iter().any(|b| strip_ci(&loc, b).map(|r| r.components().count() == 1).unwrap_or(false));
            let movable = ok && !denied(&loc) && (under_pf || strip_ci(&loc, &prof).is_some());
            AppRow { id: a.id, name: a.name, version: a.version, publisher: a.publisher, location: a.location, bytes, installed: a.installed, last_used: last, movable }
        }).collect()
    })
    .await
}
#[tauri::command]
pub async fn uninstall_app(id: String) -> Result<(), String> {
    blocking(move || -> Result<(), String> {
        let a = read_apps().into_iter().find(|a| a.id == id).ok_or("App not found")?;
        Command::new("cmd").raw_arg(format!("/C \"{}\"", a.uninstall)).spawn().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await?
}
/// Services running from inside `folder`: moving the folder under them is unsafe.
pub fn services_in(folder: &Path) -> Vec<String> {
    let f = folder.to_string_lossy().to_lowercase().replace('\'', "''");
    run_ps(&format!("$f='{f}'; Get-CimInstance Win32_Service | Where-Object {{ $_.State -eq 'Running' -and $_.PathName -and $_.PathName.ToLower().Contains($f) }} | ForEach-Object {{ $_.Name }}"))
        .unwrap_or_default().lines().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

// ---------- growth history ----------
#[derive(Serialize, Deserialize, Clone)]
struct Snap {
    when: u64,
    c_free: u64,
    c_total: u64,
    sizes: BTreeMap<String, u64>,
}
fn history_path() -> PathBuf {
    lighten_dir().join("history.json")
}
fn load_history() -> Vec<Snap> {
    fs::read_to_string(history_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}
pub fn record_snapshot(force: bool) {
    let mut h = load_history();
    if !force && h.last().map(|s| now().saturating_sub(s.when) < 6 * 3600).unwrap_or(false) {
        return;
    }
    let prof = envp("USERPROFILE");
    let mut paths: Vec<(String, PathBuf)> = vec![];
    for (label, base) in [("~", prof.clone()), ("Local", envp("LOCALAPPDATA")), ("Roaming", envp("APPDATA"))] {
        if let Ok(rd) = fs::read_dir(&base) {
            for e in rd.flatten() {
                let p = e.path();
                let Ok(md) = fs::symlink_metadata(&p) else { continue };
                let n = e.file_name().to_string_lossy().to_string();
                if md.is_dir() && !is_reparse(&md) && !(label == "~" && n.eq_ignore_ascii_case("AppData")) {
                    paths.push((format!("{label}\\{n}"), p));
                }
            }
        }
    }
    for p in [envp("SystemRoot"), envp("ProgramFiles"), envp("ProgramFiles(x86)"), envp("ProgramData")] {
        if p.is_dir() {
            paths.push((p.to_string_lossy().to_string(), p));
        }
    }
    let ps: Vec<PathBuf> = paths.iter().map(|p| p.1.clone()).collect();
    let sizes = sizes_parallel(&ps, 0);
    let c = drives().into_iter().find(|d| d.letter.eq_ignore_ascii_case("C:"));
    h.push(Snap { when: now(), c_free: c.as_ref().map(|d| d.free).unwrap_or(0), c_total: c.as_ref().map(|d| d.total).unwrap_or(0), sizes: paths.into_iter().zip(sizes).map(|(p, s)| (p.0, s.bytes)).collect() });
    let n = h.len();
    if n > 120 {
        h.drain(0..n - 120);
    }
    let _ = fs::write(history_path(), serde_json::to_string(&h).unwrap());
}
#[tauri::command]
pub async fn snapshot(force: bool) -> Result<(), String> {
    blocking(move || record_snapshot(force)).await
}
#[derive(Serialize)]
pub struct Growth {
    series: Vec<(u64, u64)>,
    growers: Vec<(String, i64, u64)>,
    days: u64,
    /// Free space lost since its highest reading in the last 14 days, and when that was
    pub(crate) drop: u64,
    since: u64,
    /// Part of the drop that the tracked folders do not explain
    unexplained: u64,
}
#[tauri::command]
pub fn growth() -> Growth {
    let h = load_history();
    let series = h.iter().map(|s| (s.when, s.c_free)).collect();
    let mut growers = vec![];
    let mut days = 0;
    let (mut drop, mut since, mut unexplained) = (0, 0, 0);
    if let Some(last) = h.last() {
        if let Some(peak) = h.iter().filter(|s| s.when < last.when && last.when - s.when <= 14 * 86400).max_by_key(|s| s.c_free) {
            drop = peak.c_free.saturating_sub(last.c_free);
            since = peak.when;
            let grew: u64 = last.sizes.iter().map(|(k, v)| v.saturating_sub(*peak.sizes.get(k).unwrap_or(v))).sum();
            unexplained = drop.saturating_sub(grew);
        }
        let base = h.iter().find(|s| last.when.saturating_sub(s.when) <= 10 * 86400 && s.when < last.when).or(h.first().filter(|s| s.when < last.when));
        if let Some(b) = base {
            days = (last.when - b.when) / 86400;
            for (k, v) in &last.sizes {
                let d = *v as i64 - *b.sizes.get(k).unwrap_or(&0) as i64;
                if d > 50 * 1024 * 1024 {
                    growers.push((k.clone(), d, *v));
                }
            }
            growers.sort_by(|a, b| b.1.cmp(&a.1));
            growers.truncate(8);
        }
    }
    Growth { series, growers, days, drop, since, unexplained }
}

// ---------- report ----------
fn ymd(t: u64) -> String {
    let z = (t / 86400) as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{m:02}-{d:02}", yoe + era * 400 + if m <= 2 { 1 } else { 0 })
}
fn gb(b: u64) -> String {
    format!("{:.2} GB", b as f64 / 1073741824.0)
}
/// A dry run: what a clean would touch, written to a text file. Nothing is deleted.
#[tauri::command]
pub async fn export_report() -> Result<String, String> {
    blocking(|| -> Result<String, String> {
        let mut r = format!("Lighten report {}\r\n\r\n", ymd(now()));
        if let Some(c) = drives().iter().find(|d| d.letter.eq_ignore_ascii_case("C:")) {
            r += &format!("C: {} free of {}\r\n\r\n", gb(c.free), gb(c.total));
        }
        r += "Would clean (dry run, nothing deleted):\r\n";
        for c in scan_cats("C:").iter().filter(|c| c.bytes > 0) {
            r += &format!("  {:>10}  {}{}\r\n", gb(c.bytes), c.name, c.busy.as_ref().map(|b| format!("  [skipped: {b} running]")).unwrap_or_default());
        }
        r += "\r\nLinks made by Lighten:\r\n";
        for m in load_journal() {
            r += &format!("  {} -> {} ({})\r\n", m.src, m.dest, gb(m.bytes));
        }
        let path = envp("USERPROFILE").join("Desktop").join(format!("Lighten-report-{}.txt", ymd(now())));
        fs::write(&path, r).map_err(|e| e.to_string())?;
        Ok(path.to_string_lossy().into())
    })
    .await?
}

// ---------- toast ----------
pub fn toast(title: &str, text: &str) {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('\'', "''");
    let script = format!(
        "[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null; [Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime] | Out-Null; $x = New-Object Windows.Data.Xml.Dom.XmlDocument; $x.LoadXml('<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>'); [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('{{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}}\\WindowsPowerShell\\v1.0\\powershell.exe').Show([Windows.UI.Notifications.ToastNotification]::new($x))",
        esc(title), esc(text)
    );
    let _ = run_ps(&script);
}

// ---------- update signing (ed25519) ----------
const PUBKEY: &str = include_str!("../update.pub");
fn unhex(s: &str) -> Result<Vec<u8>, String> {
    let s = s.trim();
    if s.len() % 2 != 0 || !s.is_ascii() { return Err("bad hex".into()); }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| e.to_string())).collect()
}
fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
pub fn signing_on() -> bool {
    !PUBKEY.trim().is_empty()
}
/// Signed text: "<version>\n<sha256 of the full exe>". When a public key is built in, unsigned or badly signed manifests are refused.
pub fn verify_signature(version: &str, full_sha: &str, sig_hex: &str) -> Result<(), String> {
    if !signing_on() {
        return Ok(());
    }
    let pk: [u8; 32] = unhex(PUBKEY)?.try_into().map_err(|_| "bad public key")?;
    let sig: [u8; 64] = unhex(sig_hex).map_err(|_| "Manifest is not signed")?.try_into().map_err(|_| "Manifest signature is malformed")?;
    VerifyingKey::from_bytes(&pk).map_err(|e| e.to_string())?
        .verify(format!("{version}\n{}", full_sha.to_lowercase()).as_bytes(), &Signature::from_bytes(&sig))
        .map_err(|_| "Update signature is invalid; not installed".to_string())
}
/// `--gen-key file`: writes the private key to `file` and the public key to `file.pub`.
pub fn gen_key(a: &[String]) {
    let res = (|| -> Result<(), String> {
        let mut seed = [0u8; 32];
        getrandom::getrandom(&mut seed).map_err(|e| e.to_string())?;
        let sk = SigningKey::from_bytes(&seed);
        fs::write(&a[0], hex(&seed)).map_err(|e| e.to_string())?;
        fs::write(format!("{}.pub", a[0]), hex(sk.verifying_key().as_bytes())).map_err(|e| e.to_string())
    })();
    if let Err(e) = res { let _ = fs::write("gen-key-error.txt", e); }
}
/// `--sign keyfile version sha256 outfile`
pub fn sign_cli(a: &[String]) {
    let res = (|| -> Result<(), String> {
        if a.len() < 4 { return Err("usage: --sign keyfile version sha256 outfile".into()); }
        let seed: [u8; 32] = unhex(&fs::read_to_string(&a[0]).map_err(|e| e.to_string())?)?.try_into().map_err(|_| "bad key file")?;
        let sig = SigningKey::from_bytes(&seed).sign(format!("{}\n{}", a[1], a[2].to_lowercase()).as_bytes());
        fs::write(&a[3], hex(&sig.to_bytes())).map_err(|e| e.to_string())
    })();
    if let Err(e) = res { let _ = fs::write("sign-error.txt", e); }
}

// ---------- installer ----------
fn install_dir() -> PathBuf {
    envp("ProgramFiles").join("Lighten")
}
#[tauri::command]
pub fn install_state() -> serde_json::Value {
    let exe = std::env::current_exe().unwrap_or_default();
    serde_json::json!({ "installed": lc(&exe).starts_with(&lc(&install_dir())), "signing": signing_on() })
}
fn q(p: &Path) -> String {
    p.to_string_lossy().replace('\'', "''")
}
#[tauri::command]
pub async fn install_app(app: AppHandle) -> Result<(), String> {
    blocking(move || -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let dir = install_dir();
        let dst = dir.join("Lighten.exe");
        let script = format!(
            "$ErrorActionPreference='Stop'\r\nNew-Item -ItemType Directory -Force '{dir}' | Out-Null\r\nif ('{src}' -ne '{dst}') {{ Copy-Item -LiteralPath '{src}' -Destination '{dst}' -Force }}\r\n$s=(New-Object -ComObject WScript.Shell).CreateShortcut(\"$env:ProgramData\\Microsoft\\Windows\\Start Menu\\Programs\\Lighten.lnk\"); $s.TargetPath='{dst}'; $s.IconLocation='{dst},0'; $s.Save()\r\n$k='HKLM:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Lighten'; New-Item -Path $k -Force | Out-Null\r\nSet-ItemProperty $k DisplayName 'Lighten'; Set-ItemProperty $k DisplayVersion '{ver}'; Set-ItemProperty $k Publisher 'Lighten'; Set-ItemProperty $k InstallLocation '{dir}'; Set-ItemProperty $k DisplayIcon '{dst}'; Set-ItemProperty $k UninstallString '\"{dst}\" --uninstall'; Set-ItemProperty $k NoModify 1 -Type DWord; Set-ItemProperty $k NoRepair 1 -Type DWord\r\n'installed'",
            dir = q(&dir), src = q(&exe), dst = q(&dst), ver = env!("CARGO_PKG_VERSION")
        );
        let out = run_ps(&script)?;
        if !out.ends_with("installed") { return Err(tail(&out, 400)); }
        let st = load_settings();
        let _ = apply_schedule_for(&st, &dst); // re-point the schedule at the installed copy
        let _ = Command::new(&dst).spawn();
        app.exit(0);
        Ok(())
    })
    .await?
}
/// Removes the schedule, shortcut, registry entry and settings/history. Keeps moves.json so Move back still works.
pub fn do_uninstall() -> String {
    let exe = std::env::current_exe().unwrap_or_default();
    let dir = install_dir();
    let in_install = lc(&exe).starts_with(&lc(&dir));
    let ld = lighten_dir();
    let mut script = format!(
        "schtasks /Delete /TN 'Lighten scan' /F | Out-Null\r\nRemove-Item \"$env:ProgramData\\Microsoft\\Windows\\Start Menu\\Programs\\Lighten.lnk\" -Force -ErrorAction SilentlyContinue\r\nRemove-Item 'HKLM:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Lighten' -Recurse -Force -ErrorAction SilentlyContinue\r\nforeach($f in 'settings.json','history.json','last-scan.json'){{ Remove-Item (Join-Path '{ld}' $f) -Force -ErrorAction SilentlyContinue }}\r\n",
        ld = q(&ld)
    );
    if in_install {
        script += &format!("Start-Process powershell -WindowStyle Hidden -ArgumentList '-NoProfile','-Command','Start-Sleep 3; Remove-Item -LiteralPath ''{}'' -Recurse -Force'\r\n", q(&dir));
    }
    let _ = run_ps(&script);
    "Uninstalled".into()
}
#[tauri::command]
pub async fn uninstall_self(app: AppHandle) -> Result<String, String> {
    let r = blocking(do_uninstall).await?;
    app.exit(0);
    Ok(r)
}
