//! NTFS fast scan, OneDrive, startup apps, extra finders, undo log, exclusions.
//! Same rules as tools.rs: deletes only act on paths a finder just offered, never protected or excluded ones.
use super::more::{log_action, note_hidden, Job};
use super::tools::{hit, mtime, offer, offered, protected, root_ok, run_ps, run_ps_input, ps_quote, tail, walk_files, DupGroup, FileHit, READ_LINES};
use super::*;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Mutex;

// ---------- exclusions ----------
static EXCL: Mutex<Option<Vec<(String, String)>>> = Mutex::new(None);
fn pairs(v: &[String]) -> Vec<(String, String)> {
    v.iter().map(|e| (lc(Path::new(e)), format!("{}\\", lc(Path::new(e))))).collect()
}
/// True when `p` is, or is inside, a folder the user excluded.
pub fn excluded(p: &Path) -> bool {
    let mut g = EXCL.lock().unwrap();
    let v = g.get_or_insert_with(|| pairs(&load_settings().exclusions));
    if v.is_empty() {
        return false;
    }
    let l = lc(p);
    v.iter().any(|(e, pre)| l == *e || l.starts_with(pre.as_str()))
}
#[tauri::command]
pub fn set_exclusions(list: Vec<String>) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = vec![];
    for e in list {
        let t = e.trim().trim_end_matches('\\').to_string();
        if t.is_empty() {
            continue;
        }
        if !Path::new(&t).is_absolute() && !(t.len() == 2 && t.ends_with(':')) {
            return Err(format!("Not an absolute path: {t}"));
        }
        if !out.iter().any(|o| o.eq_ignore_ascii_case(&t)) {
            out.push(t);
        }
    }
    let mut st = load_settings();
    st.exclusions = out.clone();
    write_settings(&st)?;
    *EXCL.lock().unwrap() = Some(pairs(&out));
    Ok(out)
}
#[tauri::command]
pub fn profile_dir() -> String {
    envp("USERPROFILE").to_string_lossy().into()
}

// ---------- undo log (Recycle Bin deletes made by Lighten) ----------
#[derive(Serialize, Deserialize, Clone)]
pub struct Trashed {
    path: String,
    bytes: u64,
    when: u64,
}
fn tlog_path() -> PathBuf {
    lighten_dir().join("trash-log.json")
}
fn load_tlog() -> Vec<Trashed> {
    fs::read_to_string(tlog_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}
pub fn log_trash(items: &[(String, u64)]) {
    if items.is_empty() {
        return;
    }
    let mut v = load_tlog();
    let t = now();
    v.extend(items.iter().map(|(p, b)| Trashed { path: p.clone(), bytes: *b, when: t }));
    let n = v.len();
    if n > 500 {
        v.drain(0..n - 500);
    }
    let _ = fs::write(tlog_path(), serde_json::to_string(&v).unwrap());
}
/// Original path stored in a Recycle Bin `$I` metadata file (format v1 and v2).
fn parse_i(b: &[u8]) -> Option<String> {
    let off = if g64(b, 0) == 2 { 28 } else { 24 };
    let u: Vec<u16> = b.get(off..)?.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
    if u.is_empty() { None } else { String::from_utf16(&u).ok() }
}
/// Original paths of everything in a drive's Recycle Bin; None if the bin can't be read.
fn bin_paths(letter: char) -> Option<std::collections::HashSet<String>> {
    let mut set = std::collections::HashSet::new();
    let mut readable = false;
    for sid in fs::read_dir(format!("{letter}:\\$Recycle.Bin")).ok()?.flatten() {
        let Ok(rd) = fs::read_dir(sid.path()) else { continue };
        readable = true;
        for f in rd.flatten() {
            if f.file_name().to_string_lossy().starts_with("$I") {
                if let Some(p) = fs::read(f.path()).ok().and_then(|b| parse_i(&b)) {
                    set.insert(lc(Path::new(&p)));
                }
            }
        }
    }
    readable.then_some(set)
}
/// Newest first; only files still in the Recycle Bin (purged ones can't be restored) and gone from their original place.
#[tauri::command]
pub fn trash_log() -> Vec<Trashed> {
    let mut bins: HashMap<char, Option<std::collections::HashSet<String>>> = HashMap::new();
    load_tlog()
        .into_iter()
        .rev()
        .filter(|t| !Path::new(&t.path).exists())
        .filter(|t| {
            let d = t.path.chars().next().unwrap_or('C').to_ascii_uppercase();
            // if a bin can't be read we keep the entry rather than hide a restorable file
            bins.entry(d).or_insert_with(|| bin_paths(d)).as_ref().map_or(true, |s| s.contains(&lc(Path::new(&t.path))))
        })
        .take(150)
        .collect()
}
#[tauri::command]
pub async fn restore_trashed(paths: Vec<String>) -> Result<u64, String> {
    blocking(move || -> Result<u64, String> {
        let log = load_tlog();
        let ok: Vec<String> = paths.into_iter().filter(|p| log.iter().any(|t| lc(Path::new(&t.path)) == lc(Path::new(p)))).collect();
        if ok.is_empty() {
            return Ok(0);
        }
        let script = format!(
            "{READ_LINES}$want=@{{}}; $lines | ForEach-Object {{ $want[$_.ToLower()]=1 }}\r\n$sh=New-Object -ComObject Shell.Application\r\nforeach($i in @($sh.NameSpace(10).Items())){{ $d=$i.ExtendedProperty('System.Recycle.DeletedFrom'); if(-not $d){{continue}}; $n=$i.Name; $e=[IO.Path]::GetExtension($i.Path); if($e -and -not $n.EndsWith($e,[StringComparison]::OrdinalIgnoreCase)){{$n+=$e}}; if($want.ContainsKey((Join-Path $d $n).ToLower())){{ $i.InvokeVerb('undelete') }} }}"
        );
        run_ps_input(&script, &ok.join("\r\n"))?;
        Ok(ok.iter().filter(|p| Path::new(p).exists()).count() as u64)
    })
    .await?
}

// ---------- NTFS fast scan: reads the master file table directly (needs administrator) ----------
struct Ent {
    parent: u32,
    name: String,
    size: u64,
    #[allow(dead_code)] // kept for tests and future sorting by age
    mtime: u64,
    attrs: u32,
    dir: bool,
}
struct Mft {
    letter: char,
    ents: Vec<Option<Box<Ent>>>, // boxed: most MFT slots are free, and an empty slot should cost 8 bytes
    tot: Vec<(u64, u64)>,
    kids: HashMap<u32, Vec<u32>>,
    when: u64,
    #[allow(dead_code)] // read by the live test
    stats: [u64; 5], // FILE signature, in use, parsed base, parsed extension, in use but unreadable
}
static MFT: Mutex<Option<Mft>> = Mutex::new(None);

fn g16(b: &[u8], i: usize) -> u16 {
    b.get(i..i + 2).map(|x| u16::from_le_bytes([x[0], x[1]])).unwrap_or(0)
}
fn g32(b: &[u8], i: usize) -> u32 {
    b.get(i..i + 4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]])).unwrap_or(0)
}
fn g64(b: &[u8], i: usize) -> u64 {
    b.get(i..i + 8).map(|x| u64::from_le_bytes([x[0], x[1], x[2], x[3], x[4], x[5], x[6], x[7]])).unwrap_or(0)
}
fn read_at(f: &mut fs::File, off: u64, len: usize) -> Result<Vec<u8>, String> {
    f.seek(SeekFrom::Start(off)).map_err(|e| e.to_string())?;
    let mut b = vec![0u8; len];
    f.read_exact(&mut b).map_err(|e| e.to_string())?;
    Ok(b)
}
/// Undoes NTFS's update-sequence protection; false if the record is damaged or not a FILE record.
fn fixup(rec: &mut [u8]) -> bool {
    if rec.len() < 48 || &rec[0..4] != b"FILE" {
        return false;
    }
    let (uo, uc) = (g16(rec, 4) as usize, g16(rec, 6) as usize);
    if uc < 2 || uo + uc * 2 > rec.len() {
        return false;
    }
    let usn = [rec[uo], rec[uo + 1]];
    for i in 1..uc {
        let end = i * 512;
        if end > rec.len() || rec[end - 2..end] != usn {
            return false;
        }
        rec[end - 2] = rec[uo + i * 2];
        rec[end - 1] = rec[uo + i * 2 + 1];
    }
    true
}
/// Calls `f(type, offset, length)` for every attribute of a fixed-up record.
fn each_attr(rec: &[u8], mut f: impl FnMut(u32, usize, usize)) {
    let mut p = g16(rec, 0x14) as usize;
    while p + 16 <= rec.len() {
        let ty = g32(rec, p);
        let len = g32(rec, p + 4) as usize;
        if ty == 0xFFFF_FFFF || len < 16 || p + len > rec.len() {
            break;
        }
        f(ty, p, len);
        p += len;
    }
}
fn parse_runs(b: &[u8]) -> Vec<(i64, u64)> {
    let (mut i, mut lcn, mut out) = (0usize, 0i64, vec![]);
    while i < b.len() && b[i] != 0 {
        let (ls, os) = ((b[i] & 0xF) as usize, (b[i] >> 4) as usize);
        i += 1;
        if ls == 0 || ls > 8 || os > 8 || i + ls + os > b.len() {
            break;
        }
        let len = b[i..i + ls].iter().rev().fold(0u64, |a, x| (a << 8) | *x as u64);
        i += ls;
        if os == 0 {
            out.push((-1, len)); // sparse
        } else {
            let mut off = b[i..i + os].iter().rev().fold(0i64, |a, x| (a << 8) | *x as i64);
            if b[i + os - 1] & 0x80 != 0 && os < 8 {
                off -= 1i64 << (8 * os); // sign-extend
            }
            lcn += off;
            out.push((lcn, len));
        }
        i += os;
    }
    out
}
fn ft(t: u64) -> u64 {
    (t / 10_000_000).saturating_sub(11_644_473_600)
}
fn utf16(b: &[u8]) -> String {
    String::from_utf16_lossy(&b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>())
}

enum Rec {
    Base(Ent),
    Ext(u32, u64),
}
/// Parses one MFT record: name, parent, on-disk size, modified time and attributes. None for unused or damaged records.
fn parse_record(rec: &mut [u8]) -> Option<Rec> {
    if !fixup(rec) || g16(rec, 0x16) & 1 == 0 {
        return None;
    }
    let flags = g16(rec, 0x16);
    let base = (g64(rec, 0x20) & 0xFFFF_FFFF_FFFF) as u32;
    let rec: &[u8] = rec;
    let (mut name, mut parent, mut size, mut mt, mut at) = (None::<String>, 5u32, None::<u64>, 0u64, 0u32);
    let mut best_ns = 9u8;
    each_attr(rec, |ty, p, len| {
        let nonres = rec[p + 8] != 0;
        match ty {
            0x10 if !nonres => {
                let c = p + g16(rec, p + 0x14) as usize;
                mt = ft(g64(rec, c + 8));
                at = g32(rec, c + 0x20);
            }
            0x30 if !nonres => {
                let c = p + g16(rec, p + 0x14) as usize;
                let (nl, ns) = (rec.get(c + 0x40).copied().unwrap_or(0) as usize, rec.get(c + 0x41).copied().unwrap_or(2));
                // prefer Win32 (1/3) over POSIX (0); skip DOS-only 8.3 names (2)
                let rank = match ns { 1 | 3 => 0, 0 => 1, _ => 9 };
                if rank < best_ns && c + 0x42 + nl * 2 <= p + len {
                    best_ns = rank;
                    parent = (g64(rec, c) & 0xFFFF_FFFF_FFFF) as u32;
                    name = Some(utf16(&rec[c + 0x42..c + 0x42 + nl * 2]));
                }
            }
            0x80 if rec[p + 9] == 0 => {
                if !nonres {
                    size = Some(g32(rec, p + 0x10) as u64);
                } else if g64(rec, p + 0x10) == 0 {
                    // on-disk size: compressed size when compressed, else allocated (0 for cloud placeholders)
                    size = Some(if g16(rec, p + 0x22) != 0 && len >= 0x48 { g64(rec, p + 0x40) } else { g64(rec, p + 0x28) });
                }
            }
            _ => {}
        }
    });
    if base != 0 {
        return size.map(|s| Rec::Ext(base, s));
    }
    Some(Rec::Base(Ent { parent, name: name?, size: size.unwrap_or(0), mtime: mt, attrs: at, dir: flags & 2 != 0 }))
}

fn build_mft(letter: char, job: Job) -> Result<Mft, String> {
    let mut f = fs::File::open(format!(r"\\.\{letter}:")).map_err(|e| format!("Cannot read the volume (needs administrator): {e}"))?;
    let boot = read_at(&mut f, 0, 512)?;
    if &boot[3..11] != b"NTFS    " {
        return Err("Not an NTFS drive".into());
    }
    let bps = g16(&boot, 0x0B) as u64;
    let cl = bps * boot[0x0D] as u64;
    let cpr = boot[0x40] as i8;
    let rs = if cpr > 0 { cpr as u64 * cl } else { 1u64 << (-(cpr as i32)) } as usize;
    if bps < 512 || cl == 0 || rs < 512 || rs > 65536 {
        return Err("Unsupported NTFS layout".into());
    }
    let align = |n: usize| n.div_ceil(bps as usize) * bps as usize;
    let mut rec0 = read_at(&mut f, g64(&boot, 0x30) * cl, align(rs))?;
    rec0.truncate(rs);
    if !fixup(&mut rec0) {
        return Err("The master file table looks damaged; scan skipped".into());
    }
    let mut data: Option<(Vec<(i64, u64)>, u64)> = None;
    each_attr(&rec0, |ty, p, len| {
        if ty == 0x80 && rec0[p + 8] != 0 && data.is_none() {
            let ro = g16(&rec0, p + 0x20) as usize;
            if ro < len {
                data = Some((parse_runs(&rec0[p + ro..p + len]), g64(&rec0, p + 0x30)));
            }
        }
    });
    let (runs, real) = data.ok_or("Could not locate the file table")?;
    let total = ((real / rs as u64) as usize).min(12_000_000);
    let mut ents: Vec<Option<Box<Ent>>> = Vec::new();
    ents.resize_with(total, || None);
    let mut ext: Vec<(u32, u64)> = vec![];
    let step = align((4 << 20) / rs * rs).max(align(rs));
    let mut idx = 0usize;
    let mut stats = [0u64; 5];
    job.phase(total as u64);
    'outer: for (lcn, len) in runs {
        let run_bytes = len * cl;
        let mut done = 0u64;
        while done < run_bytes {
            job.check()?;
            if idx >= total {
                break 'outer;
            }
            let want = ((run_bytes - done).min(step as u64)) as usize;
            if lcn < 0 {
                idx += want / rs;
                done += want as u64;
                continue;
            }
            let mut buf = read_at(&mut f, lcn as u64 * cl + done, want)?;
            for rec in buf.chunks_mut(rs) {
                if idx >= total {
                    break;
                }
                let i = idx;
                idx += 1;
                let sig = &rec[0..4] == b"FILE";
                let used = sig && g16(rec, 0x16) & 1 == 1;
                stats[0] += sig as u64;
                stats[1] += used as u64;
                match parse_record(rec) {
                    Some(Rec::Ext(base, s)) => { stats[3] += 1; ext.push((base, s)) }
                    Some(Rec::Base(e)) => { stats[2] += 1; ents[i] = Some(Box::new(e)) }
                    None => stats[4] += used as u64,
                }
            }
            done += want as u64;
            job.tick((want / rs) as u64);
        }
    }
    for (b, s) in ext {
        if let Some(Some(e)) = ents.get_mut(b as usize) {
            e.size = e.size.max(s);
        }
    }
    let n = ents.len();
    let mut tot = vec![(0u64, 0u64); n];
    let mut kids: HashMap<u32, Vec<u32>> = HashMap::new();
    for i in 0..n {
        let Some(e) = &ents[i] else { continue };
        if i == 5 {
            continue;
        }
        kids.entry(e.parent).or_default().push(i as u32);
        if e.dir {
            continue;
        }
        let (mut p, mut g) = (e.parent as usize, 0);
        while g < 128 && p < n {
            tot[p].0 += e.size;
            tot[p].1 += 1;
            if p == 5 {
                break;
            }
            match &ents[p] {
                Some(pe) => p = pe.parent as usize,
                None => break,
            }
            g += 1;
        }
    }
    Ok(Mft { letter, ents, tot, kids, when: now(), stats })
}
fn path_of(m: &Mft, mut i: usize) -> Option<String> {
    let mut parts = vec![];
    for _ in 0..128 {
        if i == 5 {
            parts.reverse();
            return Some(format!("{}:\\{}", m.letter, parts.join("\\")));
        }
        let e = m.ents.get(i)?.as_ref()?;
        parts.push(e.name.clone());
        i = e.parent as usize;
    }
    None
}
fn resolve(m: &Mft, rel: &str) -> Option<u32> {
    let mut cur = 5u32;
    for comp in rel.split('\\').filter(|c| !c.is_empty()) {
        let c = comp.to_lowercase();
        cur = *m.kids.get(&cur)?.iter().find(|&&k| m.ents[k as usize].as_ref().map(|e| e.name.to_lowercase() == c).unwrap_or(false))?;
    }
    Some(cur)
}

#[derive(Serialize)]
pub struct ScanInfo {
    files: u64,
    dirs: u64,
    bytes: u64,
    secs: u64,
}
#[tauri::command]
pub async fn disk_scan(drive: String) -> Result<ScanInfo, String> {
    let job = Job::start();
    blocking(move || -> Result<ScanInfo, String> {
        let l = drive_letter(&drive).ok_or("Unknown drive")?;
        let t = std::time::Instant::now();
        let m = build_mft(l, job)?;
        let dirs = m.ents.iter().flatten().filter(|e| e.dir).count() as u64;
        let files = m.ents.iter().flatten().count() as u64 - dirs;
        let bytes = m.tot.get(5).map(|t| t.0).unwrap_or(0);
        *MFT.lock().unwrap() = Some(m);
        Ok(ScanInfo { files, dirs, bytes, secs: t.elapsed().as_secs() })
    })
    .await?
}
#[tauri::command]
pub fn disk_release() {
    *MFT.lock().unwrap() = None;
}
/// Folder listing from the last fast scan, same shape as list_dir.
#[tauri::command]
pub async fn fast_dir(path: String) -> Result<Vec<Entry>, String> {
    blocking(move || -> Result<Vec<Entry>, String> {
        let g = MFT.lock().unwrap();
        let m = g.as_ref().ok_or("Run the fast scan first")?;
        if now().saturating_sub(m.when) > 3600 {
            return Err("The fast scan is over an hour old. Scan again.".into());
        }
        let p = path.trim().trim_end_matches('\\');
        if p.len() < 2 || !p.is_char_boundary(2) || p.as_bytes()[0].to_ascii_uppercase() as char != m.letter {
            return Err("Not the scanned drive".into());
        }
        let id = resolve(m, &p[2..]).ok_or("Folder is not in the scan (it may be new). Scan again.")?;
        let mut v: Vec<Entry> = m
            .kids
            .get(&id)
            .map(|k| k.as_slice())
            .unwrap_or(&[])
            .iter()
            .filter_map(|&k| {
                let e = m.ents[k as usize].as_ref()?;
                let link = e.attrs & 0x400 != 0;
                let (bytes, files) = if link { (0, 1) } else if e.dir { m.tot[k as usize] } else { (e.size, 1) };
                Some(Entry { path: format!("{p}\\{}", e.name), name: e.name.clone(), is_dir: e.dir && !link, link, bytes, files })
            })
            .collect();
        v.sort_by(|a, b| b.bytes.cmp(&a.bytes));
        Ok(v)
    })
    .await?
}
/// Big files from a fresh table read, or None if the drive is not NTFS / not readable (caller walks instead).
/// Sizes here are on-disk (allocated) bytes, so sparse or compressed files can fall below `min` while the walk would list them.
pub fn mft_big(root: &Path, min: u64, job: Job) -> Option<Vec<FileHit>> {
    let s = root.to_string_lossy();
    let l = s.chars().next()?.to_ascii_uppercase();
    if !l.is_ascii_uppercase() || s.chars().nth(1) != Some(':') {
        return None;
    }
    let m = build_mft(l, job).ok()?;
    let rl = lc(root);
    let skip = ["windows", "program files", "program files (x86)", "programdata", "windowsapps", ".git", "node_modules", "$recycle.bin", "system volume information"];
    let mut v = vec![];
    for (i, e) in m.ents.iter().enumerate() {
        if job.stopped() {
            break;
        }
        let Some(e) = e else { continue };
        if e.dir || e.size < min || e.attrs & 0x400 != 0 {
            continue;
        }
        let Some(p) = path_of(&m, i) else { continue };
        let pl = p.to_lowercase();
        if !(pl == rl || pl.starts_with(&format!("{rl}\\"))) || pl.split('\\').any(|c| skip.contains(&c)) {
            continue;
        }
        let pb = PathBuf::from(&p);
        if protected(&pb) {
            continue;
        }
        // trust the file system, not the table: the file must still exist as a plain file
        if let Ok(md) = fs::symlink_metadata(&pb) {
            if md.is_file() && !is_reparse(&md) && md.len() >= min {
                v.push(hit(&pb, &md));
            }
        }
    }
    Some(v)
}

// ---------- OneDrive: free up space (files stay in the cloud) ----------
const OD_RECALL: u32 = 0x0040_0000 | 0x0004_0000 | 0x1000; // recall-on-access, recall-on-open, offline
const OD_PINNED: u32 = 0x0008_0000;
fn od_roots() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = vec![];
    for k in ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"] {
        let p = envp(k);
        if !p.as_os_str().is_empty() && p.is_dir() && !v.iter().any(|x| lc(x) == lc(&p)) {
            v.push(p);
        }
    }
    v
}
#[derive(Serialize)]
pub struct OdScan {
    roots: Vec<String>,
    files: Vec<FileHit>,
}
#[tauri::command]
pub async fn onedrive_scan() -> Result<OdScan, String> {
    let job = Job::start();
    blocking(move || -> Result<OdScan, String> {
        let roots = od_roots();
        let mut files = vec![];
        for r in &roots {
            // placeholders are reparse points, so (unlike other scans) links are entered; depth-capped
            let mut stack = vec![(r.clone(), 0u32)];
            while let Some((d, depth)) = stack.pop() {
                job.check()?;
                job.tick(1);
                let Ok(rd) = fs::read_dir(&d) else { continue };
                for e in rd.flatten() {
                    let p = e.path();
                    let Ok(md) = fs::symlink_metadata(&p) else { continue };
                    if excluded(&p) {
                        continue;
                    }
                    if md.is_dir() {
                        if depth < 25 {
                            stack.push((p, depth + 1));
                        }
                    } else {
                        let a = md.file_attributes();
                        if a & REPARSE != 0 && a & (OD_RECALL | OD_PINNED) == 0 && md.len() >= 1 << 20 {
                            files.push(hit(&p, &md));
                        }
                    }
                }
            }
        }
        files.sort_by(|a, b| b.bytes.cmp(&a.bytes));
        files.truncate(500);
        offer(files.iter().map(|h| h.path.clone()));
        Ok(OdScan { roots: roots.iter().map(|r| r.to_string_lossy().into()).collect(), files })
    })
    .await?
}
#[derive(Serialize)]
pub struct Freed {
    freed: u64,
    done: u64,
    failed: u64,
}
#[tauri::command]
pub async fn onedrive_free(paths: Vec<String>) -> Result<Freed, String> {
    blocking(move || -> Result<Freed, String> {
        let roots = od_roots();
        let ok: Vec<(PathBuf, u64)> = paths
            .iter()
            .filter(|s| offered(s))
            .filter_map(|s| {
                let p = PathBuf::from(s);
                let md = fs::symlink_metadata(&p).ok()?;
                (md.is_file() && roots.iter().any(|r| strip_ci(&p, r).is_some())).then(|| (p, md.len()))
            })
            .collect();
        let failed0 = (paths.len() - ok.len()) as u64;
        if ok.is_empty() {
            return Ok(Freed { freed: 0, done: 0, failed: failed0 });
        }
        let list = ok.iter().map(|(p, _)| p.to_string_lossy().to_string()).collect::<Vec<_>>().join("\r\n");
        // attrib +U -P is exactly what "Free up space" in Explorer does; OneDrive refuses files not yet uploaded
        run_ps_input(&format!("{READ_LINES}$lines | ForEach-Object {{ attrib.exe +U -P $_ | Out-Null }}"), &list)?;
        let (mut freed, mut done) = (0, 0);
        for (p, len) in &ok {
            if fs::symlink_metadata(p).map(|m| m.file_attributes() & OD_RECALL != 0).unwrap_or(false) {
                freed += len;
                done += 1;
            }
        }
        log_action("Freed OneDrive files", freed, done, true);
        Ok(Freed { freed, done, failed: failed0 + ok.len() as u64 - done })
    })
    .await?
}

// ---------- empty folders ----------
#[derive(Serialize)]
pub struct EmptyDir {
    path: String,
    name: String,
    modified: u64,
}
/// True when `d` holds no files anywhere below it. Pushes the biggest empty subfolders of non-empty parents to `out`.
fn empty_tree(d: &Path, depth: u32, job: Job, out: &mut Vec<PathBuf>) -> bool {
    if job.stopped() {
        return false;
    }
    job.tick(1);
    let Ok(rd) = fs::read_dir(d) else { return false };
    let (mut all, mut kids) = (true, vec![]);
    let recent = now().saturating_sub(86400);
    for e in rd.flatten() {
        let p = e.path();
        let Ok(md) = fs::symlink_metadata(&p) else { all = false; continue };
        let n = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        let keep = !md.is_dir() || is_reparse(&md) || depth >= 12 || excluded(&p) || protected(&p) || denied_leaf(&n) || n.starts_with('.') || mtime(&md) > recent;
        if keep {
            all = false;
        } else if empty_tree(&p, depth + 1, job, out) {
            kids.push(p);
        } else {
            all = false;
        }
    }
    if !all {
        out.extend(kids);
    }
    all
}
fn denied_leaf(n: &str) -> bool {
    n == "appdata" || DENY.contains(&n)
}
#[tauri::command]
pub async fn find_empty(root: String) -> Result<Vec<EmptyDir>, String> {
    let job = Job::start();
    blocking(move || -> Result<Vec<EmptyDir>, String> {
        let root = root_ok(&root)?;
        let mut v = vec![];
        empty_tree(&root, 0, job, &mut v);
        job.check()?;
        let mut out: Vec<EmptyDir> = v
            .into_iter()
            .map(|p| EmptyDir { name: p.file_name().map(|n| n.to_string_lossy().into()).unwrap_or_default(), modified: fs::symlink_metadata(&p).map(|m| mtime(&m)).unwrap_or(0), path: p.to_string_lossy().into() })
            .collect();
        out.sort_by(|a, b| a.path.to_lowercase().cmp(&b.path.to_lowercase()));
        out.truncate(500);
        offer(out.iter().map(|h| h.path.clone()));
        Ok(out)
    })
    .await?
}
/// Removes `d` and empty subfolders; fs::remove_dir refuses any folder that gained a file.
fn rm_empty(d: &Path) -> bool {
    if let Ok(rd) = fs::read_dir(d) {
        for e in rd.flatten() {
            let p = e.path();
            if fs::symlink_metadata(&p).map(|m| m.is_dir() && !is_reparse(&m)).unwrap_or(false) {
                rm_empty(&p);
            }
        }
    }
    fs::remove_dir(d).is_ok()
}
#[tauri::command]
pub async fn delete_empty(paths: Vec<String>) -> Result<Freed, String> {
    blocking(move || {
        let (mut done, mut failed) = (0, 0);
        for s in paths {
            let p = PathBuf::from(&s);
            let ok = offered(&s) && fs::symlink_metadata(&p).map(|m| m.is_dir() && !is_reparse(&m)).unwrap_or(false) && !protected(&p);
            if ok && rm_empty(&p) { done += 1 } else { failed += 1 }
        }
        Freed { freed: 0, done, failed }
    })
    .await
}

// ---------- broken shortcuts ----------
#[derive(Serialize)]
pub struct LnkHit {
    path: String,
    name: String,
    bytes: u64,
    modified: u64,
    target: String,
}
#[tauri::command]
pub async fn find_shortcuts() -> Result<Vec<LnkHit>, String> {
    blocking(|| -> Result<Vec<LnkHit>, String> {
        let prof = envp("USERPROFILE");
        let mut dirs = vec![prof.join("Desktop"), PathBuf::from(r"C:\Users\Public\Desktop"), envp("APPDATA").join(r"Microsoft\Windows\Start Menu\Programs")];
        let od = envp("OneDrive");
        if !od.as_os_str().is_empty() {
            dirs.push(od.join("Desktop"));
        }
        let mut found = vec![];
        for d in &dirs {
            super::tools::find_ext(d, "lnk", 6, &mut found);
        }
        found.retain(|p| !excluded(p));
        if found.is_empty() {
            return Ok(vec![]);
        }
        let list = found.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>().join("\r\n");
        let out = run_ps_input(&format!("{READ_LINES}$sh=New-Object -ComObject WScript.Shell; $lines | ForEach-Object {{ try {{ $t=$sh.CreateShortcut($_).TargetPath }} catch {{ $t='' }}; if($t){{ $t=[Environment]::ExpandEnvironmentVariables($t); \"$_`t$t\" }} }}"), &list);
        let mut v = vec![];
        for line in out?.lines() {
            let Some((lnk, target)) = line.split_once('\t') else { continue };
            let t = target.trim();
            let tl = t.to_lowercase();
            let drive_ok = t.chars().nth(1) == Some(':') && t.chars().next().map(|c| Path::new(&format!("{}:\\", c)).exists()).unwrap_or(false);
            // only a local target on a drive that is present; never network paths, store apps or shell items
            if !drive_ok || tl.contains("windowsapps") || Path::new(t).exists() {
                continue;
            }
            let p = PathBuf::from(lnk.trim());
            if let Ok(md) = fs::symlink_metadata(&p) {
                v.push(LnkHit { path: p.to_string_lossy().into(), name: p.file_name().map(|n| n.to_string_lossy().into()).unwrap_or_default(), bytes: md.len(), modified: mtime(&md), target: t.to_string() });
            }
        }
        offer(v.iter().map(|h| h.path.clone()));
        Ok(v)
    })
    .await?
}

// ---------- similar photos (perceptual dHash) ----------
fn dhash(p: &Path) -> Option<u64> {
    let img = image::open(p).ok()?;
    let g = img.resize_exact(9, 8, image::imageops::FilterType::Triangle).to_luma8();
    let mut h = 0u64;
    for y in 0..8 {
        for x in 0..8 {
            h = (h << 1) | (g.get_pixel(x, y)[0] > g.get_pixel(x + 1, y)[0]) as u64;
        }
    }
    Some(h)
}
fn uf_find(p: &mut [usize], mut x: usize) -> usize {
    while p[x] != x {
        p[x] = p[p[x]];
        x = p[x];
    }
    x
}
#[tauri::command]
pub async fn find_similar(root: String, max_dist: u32, cap: Option<usize>) -> Result<Vec<DupGroup>, String> {
    let cap = cap.unwrap_or(6000).clamp(100, 100_000);
    let job = Job::start();
    blocking(move || -> Result<Vec<DupGroup>, String> {
        let root = root_ok(&root)?;
        let mut files: Vec<FileHit> = vec![];
        walk_files(&root, false, job, &mut |p, md| {
            let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            if ["jpg", "jpeg", "png", "gif", "bmp", "webp"].contains(&ext.as_str()) && (50_000..=60_000_000).contains(&md.len()) && files.len() < cap {
                files.push(hit(p, md));
            }
        });
        let n = files.len();
        let mut hashes: Vec<Option<u64>> = vec![None; n];
        let chunk = (n / 8).max(1);
        job.phase(n as u64);
        std::thread::scope(|s| {
            for (fs_, hs) in files.chunks(chunk).zip(hashes.chunks_mut(chunk)) {
                s.spawn(move || {
                    for (f, h) in fs_.iter().zip(hs.iter_mut()) {
                        if job.stopped() {
                            break;
                        }
                        job.tick(1);
                        *h = dhash(Path::new(&f.path));
                    }
                });
            }
        });
        // flat images hash to almost nothing and would all "match"; leave them out
        let idx: Vec<usize> = (0..n).filter(|&i| hashes[i].map(|h| (6..=58).contains(&h.count_ones())).unwrap_or(false)).collect();
        let mut par: Vec<usize> = (0..n).collect();
        let d = max_dist.min(12);
        for a in 0..idx.len() {
            job.check()?;
            for b in a + 1..idx.len() {
                if (hashes[idx[a]].unwrap() ^ hashes[idx[b]].unwrap()).count_ones() <= d {
                    let (ra, rb) = (uf_find(&mut par, idx[a]), uf_find(&mut par, idx[b]));
                    par[ra] = rb;
                }
            }
        }
        let mut groups: HashMap<usize, Vec<FileHit>> = HashMap::new();
        for &i in &idx {
            let r = uf_find(&mut par, i);
            groups.entry(r).or_default().push(files[i].clone());
        }
        let mut out: Vec<DupGroup> = groups
            .into_values()
            .filter(|g| g.len() > 1)
            .map(|mut g| {
                g.sort_by(|a, b| b.bytes.cmp(&a.bytes)); // largest first: the one to keep
                DupGroup { bytes: g[0].bytes, files: g }
            })
            .collect();
        out.sort_by(|a, b| {
            let w = |g: &DupGroup| g.files.iter().skip(1).map(|f| f.bytes).sum::<u64>();
            w(b).cmp(&w(a))
        });
        out.truncate(100);
        job.check()?;
        offer(out.iter().flat_map(|g| g.files.iter().map(|f| f.path.clone())));
        Ok(out)
    })
    .await?
}

// ---------- startup apps ----------
#[derive(Serialize)]
pub struct StartupItem {
    id: String,
    scope: String,
    kind: String,
    name: String,
    command: String,
    enabled: bool,
}
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";
fn approved_key(scope: &str, kind: &str) -> Option<String> {
    let hive = match scope { "HKCU" => "HKCU", "HKLM" => "HKLM", _ => return None };
    let leaf = match kind { "Run" => "Run", "Run32" => "Run32", "StartupFolder" => "StartupFolder", _ => return None };
    Some(format!("{hive}:\\{APPROVED}\\{leaf}"))
}
fn startup_all() -> Vec<StartupItem> {
    let script = r#"$out=@()
$defs=@(@('HKCU','Run','HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'),@('HKLM','Run','HKLM:\Software\Microsoft\Windows\CurrentVersion\Run'),@('HKLM','Run32','HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Run'))
foreach($d in $defs){ $k=Get-Item $d[2] -ErrorAction SilentlyContinue; if(-not $k){continue}
  $ap=Get-Item ($d[0]+':\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\'+$d[1]) -ErrorAction SilentlyContinue
  foreach($n in $k.GetValueNames()){ if(-not $n){continue}; $en=$true; if($ap){ $v=$ap.GetValue($n); if($v -is [byte[]] -and $v.Length -gt 0 -and ($v[0] -band 1) -eq 1){$en=$false} }
    $out+=[pscustomobject]@{scope=$d[0];kind=$d[1];name=$n;command=[string]$k.GetValue($n);enabled=$en} } }
$fd=@(@('HKCU',(Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Startup')),@('HKLM',(Join-Path $env:ProgramData 'Microsoft\Windows\Start Menu\Programs\StartUp')))
foreach($d in $fd){ if(-not (Test-Path $d[1])){continue}
  $ap=Get-Item ($d[0]+':\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\StartupFolder') -ErrorAction SilentlyContinue
  foreach($f in Get-ChildItem $d[1] -File -Force | Where-Object { $_.Name -ne 'desktop.ini' }){ $en=$true; if($ap){ $v=$ap.GetValue($f.Name); if($v -is [byte[]] -and $v.Length -gt 0 -and ($v[0] -band 1) -eq 1){$en=$false} }
    $out+=[pscustomobject]@{scope=$d[0];kind='StartupFolder';name=$f.Name;command=$f.FullName;enabled=$en} } }
if($out.Count -eq 0){ '[]' } else { ConvertTo-Json -InputObject @($out) -Compress }"#;
    let Ok(out) = run_ps(script) else { return vec![] };
    let Ok(serde_json::Value::Array(a)) = serde_json::from_str::<serde_json::Value>(&out) else { return vec![] };
    let s = |x: &serde_json::Value, k: &str| x.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    a.iter()
        .map(|x| StartupItem { id: format!("{}|{}|{}", s(x, "scope"), s(x, "kind"), s(x, "name")), scope: s(x, "scope"), kind: s(x, "kind"), name: s(x, "name"), command: s(x, "command"), enabled: x.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true) })
        .collect()
}
#[tauri::command]
pub async fn startup_list() -> Result<Vec<StartupItem>, String> {
    blocking(startup_all).await
}
/// Flips the same switch Task Manager's Startup tab uses (StartupApproved). Nothing is deleted.
#[tauri::command]
pub async fn startup_set(id: String, enabled: bool) -> Result<(), String> {
    blocking(move || -> Result<(), String> {
        let it = startup_all().into_iter().find(|i| i.id == id).ok_or("That startup entry no longer exists")?;
        let key = approved_key(&it.scope, &it.kind).ok_or("Unknown startup location")?;
        let first = if enabled { 2 } else { 3 };
        let out = run_ps(&format!(
            "$p='{}'; if(-not (Test-Path $p)){{ New-Item -Path $p -Force | Out-Null }}; New-ItemProperty -Path $p -Name '{}' -PropertyType Binary -Value ([byte[]]({first},0,0,0,0,0,0,0,0,0,0,0)) -Force | Out-Null; 'ok'",
            ps_quote(&key), ps_quote(&it.name)
        ))?;
        if out.ends_with("ok") { Ok(()) } else { Err(tail(&out, 300)) }
    })
    .await?
}

// ---------- component store (WinSxS) report ----------
#[tauri::command]
pub async fn winsxs_analyze() -> Result<Vec<(String, String)>, String> {
    blocking(|| -> Result<Vec<(String, String)>, String> {
        let o = run("dism", &["/Online", "/Cleanup-Image", "/AnalyzeComponentStore"])?;
        let s = String::from_utf8_lossy(&o.stdout).to_string();
        let kv: Vec<(String, String)> = s.lines().filter_map(|l| l.split_once(" : ")).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect();
        if kv.is_empty() { Err(tail(&s, 300)) } else { Ok(kv) }
    })
    .await?
}


#[cfg(test)]
#[path = "extra_tests.rs"]
mod tests;

// ---------- hidden space: files the file table lists but Windows cannot find ----------
#[derive(Serialize)]
pub struct Hidden {
    path: String,
    bytes: u64,
}
/// Big files that exist in the NTFS table but report "not found" when opened by path.
/// They hold real space (free space shrinks) yet no folder listing shows them.
fn find_hidden(m: &Mft) -> Vec<Hidden> {
    let mut v = vec![];
    for (i, e) in m.ents.iter().enumerate() {
        let Some(e) = e else { continue };
        if e.dir || e.size < 200 << 20 {
            continue;
        }
        let Some(p) = path_of(m, i) else { continue };
        if p[3..].starts_with('$') {
            continue; // $MFT, $LogFile and friends are not reachable by name
        }
        // PermissionDenied (WindowsApps and the like) is a file we merely cannot read; only NotFound is hidden
        if matches!(fs::symlink_metadata(&p), Err(x) if x.kind() == std::io::ErrorKind::NotFound) {
            v.push(Hidden { path: p, bytes: e.size });
        }
    }
    v.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    v.truncate(20);
    v
}
#[tauri::command]
pub async fn hidden_space() -> Result<Vec<Hidden>, String> {
    blocking(|| -> Result<Vec<Hidden>, String> {
        let g = MFT.lock().unwrap();
        let m = g.as_ref().ok_or("Run the fast scan first")?;
        let v = find_hidden(m);
        note_hidden(m.letter, v.len() as u64, v.iter().map(|h| h.bytes).sum());
        Ok(v)
    })
    .await?
}

/// Schedules a full check of the drive for the next restart; it can reattach lost files to a found.000 folder.
#[tauri::command]
pub async fn schedule_chkdsk(drive: String) -> Result<String, String> {
    blocking(move || -> Result<String, String> {
        let l = drive_letter(&drive).ok_or("Unknown drive")?;
        let o = sys_cmd("cmd").args(["/c", &format!("echo Y| chkdsk {l}: /f")]).output().map_err(|e| e.to_string())?;
        let s = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
        Ok(tail(s.trim(), 400))
    })
    .await?
}
