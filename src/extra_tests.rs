use super::*;
use std::os::windows::fs::OpenOptionsExt;

fn put(b: &mut [u8], i: usize, v: &[u8]) {
    b[i..i + v.len()].copy_from_slice(v);
}
/// A hand-built 1024-byte FILE record: resident $STANDARD_INFORMATION and $FILE_NAME, non-resident unnamed $DATA.
fn sample_record(flags: u16) -> Vec<u8> {
    let mut r = vec![0u8; 1024];
    put(&mut r, 0, b"FILE");
    put(&mut r, 4, &0x30u16.to_le_bytes()); // update sequence array offset
    put(&mut r, 6, &3u16.to_le_bytes()); // 1 usn + 2 sectors
    put(&mut r, 0x14, &0x38u16.to_le_bytes());
    put(&mut r, 0x16, &flags.to_le_bytes());
    let mut p = 0x38;
    // $STANDARD_INFORMATION
    put(&mut r, p, &0x10u32.to_le_bytes());
    put(&mut r, p + 4, &0x48u32.to_le_bytes());
    put(&mut r, p + 0x10, &0x30u32.to_le_bytes());
    put(&mut r, p + 0x14, &0x18u16.to_le_bytes());
    put(&mut r, p + 0x18 + 8, &((1_700_000_000u64 + 11_644_473_600) * 10_000_000).to_le_bytes());
    put(&mut r, p + 0x18 + 0x20, &0x20u32.to_le_bytes());
    p += 0x48;
    // $FILE_NAME: first a DOS-only name that must be ignored, then the Win32 name
    for (ns, name) in [(2u8, "TEST~1"), (1u8, "test.bin")] {
        let units: Vec<u8> = name.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        let len = (0x18 + 0x42 + units.len() + 7) / 8 * 8;
        put(&mut r, p, &0x30u32.to_le_bytes());
        put(&mut r, p + 4, &(len as u32).to_le_bytes());
        put(&mut r, p + 0x10, &((0x42 + units.len()) as u32).to_le_bytes());
        put(&mut r, p + 0x14, &0x18u16.to_le_bytes());
        put(&mut r, p + 0x18, &7u64.to_le_bytes()); // parent record 7
        r[p + 0x18 + 0x40] = (units.len() / 2) as u8;
        r[p + 0x18 + 0x41] = ns;
        put(&mut r, p + 0x18 + 0x42, &units);
        p += len;
    }
    // $DATA, non-resident, allocated 8192, real 5000
    put(&mut r, p, &0x80u32.to_le_bytes());
    put(&mut r, p + 4, &0x48u32.to_le_bytes());
    r[p + 8] = 1;
    put(&mut r, p + 0x20, &0x40u16.to_le_bytes());
    put(&mut r, p + 0x28, &8192u64.to_le_bytes());
    put(&mut r, p + 0x30, &5000u64.to_le_bytes());
    p += 0x48;
    put(&mut r, p, &0xFFFF_FFFFu32.to_le_bytes());
    // update sequence: usn 0x0001, saved sector tails 0
    put(&mut r, 0x30, &1u16.to_le_bytes());
    put(&mut r, 510, &1u16.to_le_bytes());
    put(&mut r, 1022, &1u16.to_le_bytes());
    r
}

#[test]
fn parses_a_record() {
    let mut r = sample_record(1);
    let Some(Rec::Base(e)) = parse_record(&mut r) else { panic!("not parsed") };
    assert_eq!((e.name.as_str(), e.parent, e.size, e.mtime, e.attrs, e.dir), ("test.bin", 7, 8192, 1_700_000_000, 0x20, false));
    assert!(parse_record(&mut sample_record(0)).is_none(), "unused record");
    let mut dir = sample_record(3);
    assert!(matches!(parse_record(&mut dir), Some(Rec::Base(e)) if e.dir));
    let mut bad = sample_record(1);
    bad[510] = 9; // torn write: update sequence mismatch
    assert!(parse_record(&mut bad).is_none());
    let mut short = vec![0u8; 20];
    assert!(parse_record(&mut short).is_none());
}
#[test]
fn data_runs() {
    assert_eq!(parse_runs(&[0x21, 0x18, 0x34, 0x56, 0x11, 0x10, 0xF0, 0x00]), vec![(0x5634, 0x18), (0x5624, 0x10)]);
    assert_eq!(parse_runs(&[0x01, 0x05, 0x00]), vec![(-1, 5)]); // sparse
    assert!(parse_runs(&[0x41, 0x05]).is_empty()); // truncated, must not panic
}
fn backdate(p: &Path) {
    let old = SystemTime::now() - std::time::Duration::from_secs(10 * 86400);
    let f = fs::OpenOptions::new().access_mode(0x100).share_mode(7).custom_flags(0x0200_0000).open(p).unwrap();
    f.set_modified(old).unwrap();
}
static LOCK: Mutex<()> = Mutex::new(()); // the exclusion list is global
fn set_excl_for_test(v: &[String]) {
    *EXCL.lock().unwrap() = Some(pairs(v));
}
#[test]
fn empty_folders() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let root = std::env::temp_dir().join(format!("lighten-empty-{}", now()));
    for d in ["a/b/c", "keep/sub", "e", "ex/skip"] {
        fs::create_dir_all(root.join(d)).unwrap();
    }
    fs::write(root.join("keep/file.txt"), b"x").unwrap();
    for d in ["a/b/c", "a/b", "a", "keep/sub", "keep", "e", "ex/skip", "ex"] {
        backdate(&root.join(d));
    }
    set_excl_for_test(&[root.join("ex").to_string_lossy().to_string()]);
    let mut out = vec![];
    let top = empty_tree(&root, 0, &mut out);
    set_excl_for_test(&[]);
    let mut names: Vec<String> = out.iter().map(|p| p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/")).collect();
    names.sort();
    assert!(!top);
    assert_eq!(names, vec!["a", "e", "keep/sub"], "maximal empty folders only; excluded and non-empty ones are left");
    assert!(!rm_empty(&root.join("keep")), "a folder with a file is never removed");
    assert!(rm_empty(&root.join("a")) && !root.join("a").exists());
    let _ = fs::remove_dir_all(&root);
}
#[test]
fn exclusions_match_whole_components() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    set_excl_for_test(&["C:\\Work\\Proj".to_string()]);
    assert!(excluded(Path::new("c:\\work\\proj")) && excluded(Path::new("C:\\Work\\Proj\\sub\\x.txt")));
    assert!(!excluded(Path::new("C:\\Work\\Project2")) && !excluded(Path::new("C:\\Work")));
    set_excl_for_test(&[]);
}
#[test]
fn similar_images_hash_close() {
    let mk = |w: u32, h: u32, fx: f32, fy: f32| image::DynamicImage::ImageLuma8(image::GrayImage::from_fn(w, h, |x, y| image::Luma([(128.0 + 100.0 * ((x as f32 / w as f32) * fx).sin() * ((y as f32 / h as f32) * fy).cos()) as u8])));
    let dir = std::env::temp_dir().join(format!("lighten-img-{}", now()));
    fs::create_dir_all(&dir).unwrap();
    let (a, b, c) = (dir.join("a.png"), dir.join("b.png"), dir.join("c.png"));
    mk(400, 300, 9.0, 7.0).save(&a).unwrap();
    mk(200, 150, 9.0, 7.0).save(&b).unwrap(); // same picture, resized
    mk(400, 300, 31.0, 3.0).save(&c).unwrap(); // different picture
    let (ha, hb, hc) = (dhash(&a).unwrap(), dhash(&b).unwrap(), dhash(&c).unwrap());
    assert!((ha ^ hb).count_ones() <= 4, "resized copy: {}", (ha ^ hb).count_ones());
    assert!((ha ^ hc).count_ones() > 8, "different picture: {}", (ha ^ hc).count_ones());
    let _ = fs::remove_dir_all(&dir);
}
#[test]
fn startup_entries_read() {
    let v = startup_all();
    eprintln!("{} startup entries, e.g. {:?}", v.len(), v.first().map(|s| (&s.id, s.enabled)));
    assert!(v.iter().all(|s| approved_key(&s.scope, &s.kind).is_some()));
}
/// Needs administrator: `cargo test mft_live -- --ignored` from an elevated terminal.
#[test]
#[ignore]
fn mft_live() {
    let t = std::time::Instant::now();
    let m = build_mft('C').unwrap();
    let files = m.ents.iter().flatten().filter(|e| !e.dir).count();
    let used = m.tot[5].0;
    let t_rows = format!("{} files, {} bytes in {:?}", files, used, t.elapsed());
    let nz = m.ents.iter().flatten().count();
    let root: Vec<String> = m.kids.get(&5).map(|k| k.iter().filter_map(|&i| m.ents[i as usize].as_ref().map(|e| e.name.clone())).collect()).unwrap_or_default();
    let diag = format!("stats {:?} runs-total {} slots {} parsed {} root kids {} ({:?}); ent5 {:?}", m.stats, m.ents.len(), m.ents.len(), nz, root.len(), &root[..root.len().min(40)], m.ents.get(5).and_then(|e| e.as_ref()).map(|e| (e.name.clone(), e.parent)));
    let _ = fs::write(std::env::temp_dir().join("lighten-mft-diag.txt"), &diag);
    // hard-linked files (most of System32) appear under one of their names only, so check a plain file and a plain folder
    let pf = resolve(&m, "pagefile.sys").expect("pagefile in table");
    let pf_len = fs::metadata(r"C:\pagefile.sys").map(|x| x.len()).unwrap_or(0);
    let pf_sz = m.ents[pf as usize].as_ref().unwrap().size;
    let dir = resolve(&m, r"Program Files\BraveSoftware").expect("folder in table");
    let (tb, tf) = m.tot[dir as usize];
    let real = dir_size(Path::new(r"C:\Program Files\BraveSoftware"), 0);
    let line = format!("{t_rows}; pagefile table {pf_sz} vs real {pf_len}; Brave dir table {tb} B/{tf} files vs walk {} B/{} files", real.bytes, real.files);
    eprintln!("{line}");
    let _ = fs::write(std::env::temp_dir().join("lighten-mft-live.txt"), &line);
    assert!(files > 1000 && used > 1 << 30);
    assert!(pf_sz >= pf_len && pf_sz < pf_len + 65536, "on-disk size is the length rounded up to clusters");
    assert!(tf >= real.files * 9 / 10 && tf <= real.files * 11 / 10, "file count within 10% of a real walk");
    assert!(tb >= real.bytes * 9 / 10 && tb <= real.bytes + real.files * 65536, "size close to a real walk (cluster rounding adds a little)");
}

#[test]
fn recycle_bin_metadata_parses() {
    let p: Vec<u8> = r"C:\Users\a\x.txt".encode_utf16().chain([0]).flat_map(|c| c.to_le_bytes()).collect();
    let mut v2 = vec![0u8; 24];
    v2[0] = 2;
    v2.extend((p.len() as u32 / 2).to_le_bytes());
    v2.extend(&p);
    assert_eq!(parse_i(&v2).as_deref(), Some(r"C:\Users\a\x.txt"));
    let mut v1 = vec![0u8; 24];
    v1[0] = 1;
    v1.extend(&p);
    assert_eq!(parse_i(&v1).as_deref(), Some(r"C:\Users\a\x.txt"));
    assert_eq!(parse_i(&[0u8; 10]), None);
}

/// Where does the space on C: live? Elevated: `cargo test mft_top -- --ignored`; writes %TEMP%\lighten-top.txt
#[test]
#[ignore]
fn mft_top() {
    let m = build_mft('C').unwrap();
    let mut out = String::new();
    let mut top: Vec<(u64, String)> = m.kids.get(&5).into_iter().flatten().map(|&i| (m.tot[i as usize].0, path_of(&m, i as usize).unwrap_or_default())).collect();
    top.sort_by(|a, b| b.0.cmp(&a.0));
    out += "TOP LEVEL\n";
    for (b, p) in top.iter().take(15) { out += &format!("{:8.1} GB  {}\n", *b as f64 / 1e9, p); }
    let mut own: Vec<(u64, String)> = (0..m.ents.len())
        .filter(|&i| m.ents[i].as_ref().map_or(false, |e| e.dir))
        .map(|i| {
            let kids: u64 = m.kids.get(&(i as u32)).into_iter().flatten().filter(|&&k| m.ents[k as usize].as_ref().map_or(false, |e| e.dir)).map(|&k| m.tot[k as usize].0).sum();
            (m.tot[i].0.saturating_sub(kids), i)
        })
        .map(|(b, i)| (b, path_of(&m, i).unwrap_or_default()))
        .collect();
    own.sort_by(|a, b| b.0.cmp(&a.0));
    out += "\nFOLDERS BY OWN FILES\n";
    for (b, p) in own.iter().take(30) { out += &format!("{:8.1} GB  {}\n", *b as f64 / 1e9, p); }
    fs::write(std::env::temp_dir().join("lighten-top.txt"), out).unwrap();
}

#[test]
#[ignore]
fn mft_kids() {
    let m = build_mft('C').unwrap();
    let d = resolve(&m, r"Users\dhanu\AppData\Roaming\1DownloadManager\cache\work").unwrap();
    let mut out = format!("dir {} tot {:?} self attrs {:#x}\n", d, m.tot[d as usize], m.ents[d as usize].as_ref().unwrap().attrs);
    out += &format!("own size field {}\n", m.ents[d as usize].as_ref().unwrap().size);
    for &k in m.kids.get(&d).into_iter().flatten() {
        let e = m.ents[k as usize].as_ref().unwrap();
        out += &format!("{} {} size {} attrs {:#x} dir {}\n", k, e.name, e.size, e.attrs, e.dir);
    }
    fs::write(std::env::temp_dir().join("lighten-kids.txt"), out).unwrap();
}

#[test]
#[ignore]
fn mft_gap() {
    let m = build_mft('C').unwrap();
    let mut out = format!("stats {:?} (sig, in use, base, ext, unreadable) root total {}\n", m.stats, m.tot[5].0);
    let mut all = 0u64;
    let mut orphan = 0u64;
    let mut big: Vec<(u64, usize)> = vec![];
    for (i, e) in m.ents.iter().enumerate() {
        let Some(e) = e else { continue };
        if e.dir { continue }
        all += e.size;
        if path_of(&m, i).is_none() { orphan += e.size }
        big.push((e.size, i));
    }
    out += &format!("all file bytes {} orphan (no path to root) {}\n", all, orphan);
    big.sort_by(|a, b| b.0.cmp(&a.0));
    for (s, i) in big.iter().take(25) {
        out += &format!("{:8.2} GB  {}  [{}]\n", *s as f64 / 1e9, path_of(&m, *i).unwrap_or_else(|| format!("(orphan) {}", m.ents[*i].as_ref().unwrap().name)), i);
    }
    fs::write(std::env::temp_dir().join("lighten-gap.txt"), out).unwrap();
}

#[test]
fn hidden_files_are_found() {
    let ent = |parent: u32, name: &str, size: u64, dir: bool| Some(Box::new(Ent { parent, name: name.into(), size, mtime: 0, attrs: 0, dir }));
    let mut ents: Vec<Option<Box<Ent>>> = (0..10).map(|_| None).collect();
    ents[5] = ent(5, ".", 0, true);
    ents[6] = ent(5, "Windows", 0, true);
    ents[7] = ent(6, "explorer.exe", 300 << 20, false); // real file, big
    ents[8] = ent(6, "lighten-no-such-file.bin", 400 << 20, false); // listed but not on disk
    ents[9] = ent(6, "lighten-small-ghost.bin", 1 << 20, false); // too small to matter
    let m = Mft { letter: 'C', ents, tot: vec![(0, 0); 10], kids: HashMap::new(), when: 0, stats: [0; 5] };
    let h = find_hidden(&m);
    assert_eq!(h.len(), 1);
    assert!(h[0].path.ends_with("lighten-no-such-file.bin") && h[0].bytes == 400 << 20);
}

#[test]
#[ignore]
fn hidden_live() {
    let m = build_mft('C').unwrap();
    let h = find_hidden(&m);
    let out: String = h.iter().map(|x| format!("{} {}\n", x.bytes, x.path)).collect();
    fs::write(std::env::temp_dir().join("lighten-hidden.txt"), format!("{} hidden\n{out}", h.len())).unwrap();
}
