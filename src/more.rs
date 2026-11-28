//! Scan cancel and progress, cleanup history, CSV export, hidden-file memory, Explorer menu.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering::SeqCst};
use std::sync::Mutex;

// ---------- cancel and progress ----------
static SEQ: AtomicU64 = AtomicU64::new(0);
static DEAD: AtomicU64 = AtomicU64::new(0); // every job numbered <= DEAD has been cancelled
static DONE: AtomicU64 = AtomicU64::new(0);
static TOTAL: AtomicU64 = AtomicU64::new(0);
pub const CANCELLED: &str = "Scan cancelled";

/// One user-started scan. A ticket number, so cancelling an old scan can never revive or kill a later one.
#[derive(Clone, Copy)]
pub struct Job(u64);
impl Job {
    pub fn start() -> Job {
        DONE.store(0, SeqCst);
        TOTAL.store(0, SeqCst);
        Job(SEQ.fetch_add(1, SeqCst) + 1)
    }
    pub fn stopped(self) -> bool {
        self.0 <= DEAD.load(SeqCst)
    }
    pub fn check(self) -> Result<(), String> {
        if self.stopped() { Err(CANCELLED.into()) } else { Ok(()) }
    }
    /// Progress belongs to the newest scan only.
    pub fn tick(self, n: u64) {
        if self.0 == SEQ.load(SeqCst) {
            DONE.fetch_add(n, SeqCst);
        }
    }
    /// Starts a counted step; 0 means the total is unknown.
    pub fn phase(self, total: u64) {
        if self.0 == SEQ.load(SeqCst) {
            DONE.store(0, SeqCst);
            TOTAL.store(total, SeqCst);
        }
    }
}
#[tauri::command]
pub fn cancel_scan() {
    DEAD.store(SEQ.load(SeqCst), SeqCst);
}
#[tauri::command]
pub fn scan_progress() -> serde_json::Value {
    serde_json::json!({ "done": DONE.load(SeqCst), "total": TOTAL.load(SeqCst) })
}

fn read_json<T: serde::de::DeserializeOwned + Default>(name: &str) -> T {
    fs::read_to_string(lighten_dir().join(name)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}
fn write_json<T: Serialize>(name: &str, v: &T) {
    if let Ok(s) = serde_json::to_string(v) {
        let _ = fs::write(lighten_dir().join(name), s);
    }
}

// ---------- cleanup history ----------
#[derive(Serialize, Deserialize, Clone)]
pub struct Action {
    pub when: u64,
    pub what: String,
    pub bytes: u64,
    pub items: u64,
    /// false when the space is not free yet (files sitting in the Recycle Bin)
    pub freed: bool,
}
static HISTORY: Mutex<()> = Mutex::new(());
pub fn push_capped(v: &mut Vec<Action>, a: Action) {
    v.push(a);
    let n = v.len();
    if n > 200 {
        v.drain(0..n - 200);
    }
}
pub fn log_action(what: &str, bytes: u64, items: u64, freed: bool) {
    if bytes == 0 && items == 0 {
        return;
    }
    let _g = HISTORY.lock().unwrap();
    let mut v: Vec<Action> = read_json("clean-log.json");
    push_capped(&mut v, Action { when: now(), what: what.into(), bytes, items, freed });
    write_json("clean-log.json", &v);
}
/// Newest first.
#[tauri::command]
pub fn clean_history() -> Vec<Action> {
    let mut v: Vec<Action> = read_json("clean-log.json");
    v.reverse();
    v
}

// ---------- CSV export ----------
/// The real Desktop folder. It is often moved (OneDrive), so never assume it sits in the profile.
pub fn desktop(app: &AppHandle) -> Result<PathBuf, String> {
    use tauri::Manager;
    app.path().desktop_dir().map_err(|e| format!("Cannot find the Desktop folder: {e}"))
}
/// Quotes a field. A leading =, +, - or @ is defused so a file name cannot run as a spreadsheet formula.
fn csv_field(s: &str) -> String {
    let s = if s.starts_with(['=', '+', '-', '@', '\t', '\r']) { format!("'{s}") } else { s.to_string() };
    format!("\"{}\"", s.replace('"', "\"\""))
}
pub fn csv_text(rows: &[Vec<String>]) -> String {
    // the byte-order mark makes Excel read the file as UTF-8
    let mut out = String::from("\u{feff}");
    for r in rows {
        out += &r.iter().map(|f| csv_field(f)).collect::<Vec<_>>().join(",");
        out += "\r\n";
    }
    out
}
#[tauri::command]
pub async fn save_csv(app: AppHandle, kind: String, rows: Vec<Vec<String>>) -> Result<String, String> {
    let dir = desktop(&app)?;
    blocking(move || -> Result<String, String> {
        if rows.len() > 200_000 {
            return Err("Too many rows to export".into());
        }
        let kind: String = kind.chars().filter(|c| c.is_ascii_alphanumeric()).take(20).collect();
        let path = dir.join(format!("Lighten-{kind}-{}.csv", now()));
        fs::write(&path, csv_text(&rows)).map_err(|e| format!("Could not save to the Desktop: {e}"))?;
        Ok(path.to_string_lossy().into())
    })
    .await?
}

// ---------- hidden files found by the last fast scan of each drive ----------
#[derive(Serialize, Deserialize, Clone)]
pub struct HiddenNote {
    pub drive: String,
    pub when: u64,
    pub files: u64,
    pub bytes: u64,
}
pub fn merge_note(v: &mut Vec<HiddenNote>, n: HiddenNote) {
    v.retain(|x| x.drive != n.drive);
    if n.files > 0 {
        v.push(n);
    }
}
pub fn note_hidden(drive: char, files: u64, bytes: u64) {
    let mut v: Vec<HiddenNote> = read_json("hidden.json");
    merge_note(&mut v, HiddenNote { drive: format!("{drive}:"), when: now(), files, bytes });
    write_json("hidden.json", &v);
}
#[tauri::command]
pub fn last_hidden() -> Vec<HiddenNote> {
    read_json("hidden.json")
}

// ---------- Explorer right-click menu (per user, so no machine-wide change) ----------
const MENU_KEYS: [&str; 2] = [r"HKCU\Software\Classes\Directory\shell\Lighten", r"HKCU\Software\Classes\Drive\shell\Lighten"];
fn reg(args: &[&str]) -> bool {
    run("reg", args).map(|o| o.status.success()).unwrap_or(false)
}
pub fn menu_command(exe: &Path) -> String {
    format!("\"{}\" --analyze \"%1\"", exe.display())
}
pub fn menu_on() -> bool {
    MENU_KEYS.iter().all(|k| reg(&["query", &format!(r"{k}\command")]))
}
pub fn menu_write(exe: &Path, on: bool) -> Result<(), String> {
    for k in MENU_KEYS {
        if !on {
            let _ = reg(&["delete", k, "/f"]);
            continue;
        }
        let e = exe.to_string_lossy();
        let ok = reg(&["add", k, "/ve", "/d", "Analyze with Lighten", "/f"])
            && reg(&["add", k, "/v", "Icon", "/d", &e, "/f"])
            && reg(&["add", &format!(r"{k}\command"), "/ve", "/d", &menu_command(exe), "/f"]);
        if !ok {
            return Err("Could not add the right-click menu".into());
        }
    }
    Ok(())
}
#[tauri::command]
pub async fn shell_menu_state() -> Result<bool, String> {
    blocking(menu_on).await
}
#[tauri::command]
pub async fn shell_menu_set(on: bool) -> Result<bool, String> {
    blocking(move || -> Result<bool, String> {
        menu_write(&std::env::current_exe().map_err(|e| e.to_string())?, on)?;
        Ok(menu_on())
    })
    .await?
}
/// The folder after `--analyze`. Explorer passes a drive as "C:\", whose last backslash swallows the closing quote.
pub fn analyze_arg(args: &[String]) -> Option<String> {
    let i = args.iter().position(|a| a == "--analyze")?;
    let mut p = args.get(i + 1)?.trim_end_matches('"').to_string();
    if p.len() == 2 && p.ends_with(':') {
        p.push('\\');
    }
    Path::new(&p).is_absolute().then_some(p)
}
#[tauri::command]
pub fn launch_path() -> Option<String> {
    analyze_arg(&std::env::args().collect::<Vec<_>>()).filter(|p| Path::new(p).is_dir())
}

// ---------- AI model folders: sizes only, never cleaned ----------
/// Where each tool keeps downloaded models. `var` reads an environment variable, so a moved store is found.
pub fn ai_dirs(profile: &Path, var: impl Fn(&str) -> Option<String>) -> Vec<(&'static str, PathBuf)> {
    let set = |k: &str| var(k).filter(|v| !v.trim().is_empty()).map(PathBuf::from);
    let hf = set("HF_HUB_CACHE").or_else(|| set("HF_HOME").map(|h| h.join("hub"))).unwrap_or_else(|| profile.join(r".cache\huggingface\hub"));
    vec![
        ("Ollama", set("OLLAMA_MODELS").unwrap_or_else(|| profile.join(r".ollama\models"))),
        ("Hugging Face", hf),
        ("LM Studio", profile.join(r".lmstudio\models")),
        ("LM Studio (older)", profile.join(r".cache\lm-studio\models")),
    ]
}
#[derive(Serialize)]
pub struct AiStore {
    name: String,
    path: String,
    bytes: u64,
}
#[tauri::command]
pub async fn ai_models() -> Result<Vec<AiStore>, String> {
    blocking(|| {
        ai_dirs(&envp("USERPROFILE"), |k| std::env::var(k).ok())
            .into_iter()
            .filter(|(_, p)| p.is_dir())
            .map(|(name, p)| AiStore { name: name.into(), bytes: dir_size(&p, 0).bytes, path: p.to_string_lossy().into() })
            .filter(|s| s.bytes > 0)
            .collect()
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_model_folders_follow_the_tools_own_settings() {
        let at = |vars: &[(&str, &str)]| ai_dirs(Path::new(r"C:\Users\a"), |k| vars.iter().find(|v| v.0 == k).map(|v| v.1.to_string()));
        assert_eq!(at(&[])[0].1, Path::new(r"C:\Users\a\.ollama\models"));
        assert_eq!(at(&[])[1].1, Path::new(r"C:\Users\a\.cache\huggingface\hub"));
        assert_eq!(at(&[("OLLAMA_MODELS", r"D:\m"), ("HF_HOME", r"D:\hf")])[0].1, Path::new(r"D:\m"));
        assert_eq!(at(&[("HF_HOME", r"D:\hf")])[1].1, Path::new(r"D:\hf\hub"));
    }

    #[test]
    fn cancel_stops_running_jobs_but_not_later_ones() {
        let a = Job::start();
        assert!(!a.stopped());
        cancel_scan();
        assert!(a.stopped() && a.check() == Err(CANCELLED.to_string()));
        assert!(!Job::start().stopped());
    }
    #[test]
    fn csv_quotes_and_defuses_formulas() {
        let t = csv_text(&[vec!["=cmd()".into(), "a \"b\", c".into(), "C:\\x".into()]]);
        assert_eq!(t, "\u{feff}\"'=cmd()\",\"a \"\"b\"\", c\",\"C:\\x\"\r\n");
    }
    #[test]
    fn history_is_capped() {
        let mut v = vec![];
        for i in 0..205 {
            push_capped(&mut v, Action { when: i, what: "x".into(), bytes: 1, items: 1, freed: true });
        }
        assert_eq!((v.len(), v[0].when), (200, 5));
    }
    #[test]
    fn hidden_note_replaces_and_clears_per_drive() {
        let n = |d: &str, files| HiddenNote { drive: d.into(), when: 1, files, bytes: 9 };
        let mut v = vec![];
        merge_note(&mut v, n("C:", 1));
        merge_note(&mut v, n("D:", 2));
        merge_note(&mut v, n("C:", 0));
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].drive, "D:");
    }
    #[test]
    fn analyze_argument_handles_explorer_quoting() {
        let a = |s: &str| analyze_arg(&["x.exe".into(), "--analyze".into(), s.into()]);
        assert_eq!(a("C:\""), Some("C:\\".into()));
        assert_eq!(a("D:\\My Folder"), Some("D:\\My Folder".into()));
        assert_eq!(a("relative"), None);
        assert_eq!(analyze_arg(&["x.exe".into(), "--analyze".into()]), None);
    }
}
#[cfg(test)]
mod manifest_check {
    /// `LIGHTEN_MANIFEST=latest.json cargo test manifest_file -- --ignored` checks a manifest the way the app will.
    #[test]
    #[ignore]
    fn manifest_file_is_signed_for_this_key() {
        let p = std::env::var("LIGHTEN_MANIFEST").expect("set LIGHTEN_MANIFEST");
        let m: crate::Manifest = serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap();
        crate::tools::verify_signature(&m.version, &m.full.sha256, &m.signature).unwrap();
        assert!(crate::tools::verify_signature("9.9.9", &m.full.sha256, &m.signature).is_err());
    }
}
