//! Starting other programs. Lighten runs as administrator, so programs are named by full path
//! (never looked up beside the exe), and scripts and file lists are never saved to disk first:
//! anything a normal program can write to could be swapped before it runs.
use super::NO_WINDOW;
use std::ffi::OsString;
use std::io::Write;
use std::os::windows::{ffi::OsStringExt, process::CommandExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[link(name = "kernel32")]
extern "system" {
    fn GetSystemDirectoryW(buf: *mut u16, len: u32) -> u32;
}

/// Asked from Windows, not read from %SystemRoot%, which a user can override.
pub fn system32() -> PathBuf {
    let mut b = [0u16; 261];
    let n = unsafe { GetSystemDirectoryW(b.as_mut_ptr(), b.len() as u32) } as usize;
    if n == 0 || n >= b.len() {
        return PathBuf::from(r"C:\Windows\System32");
    }
    PathBuf::from(OsString::from_wide(&b[..n]))
}
fn windows_drive() -> PathBuf {
    system32().ancestors().nth(2).map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from(r"C:\"))
}
/// True for a copy only an administrator can replace.
pub fn in_program_files(exe: &Path) -> bool {
    let l = exe.to_string_lossy().to_lowercase();
    ["Program Files", "Program Files (x86)"].iter().any(|d| l.starts_with(&(windows_drive().join(d).to_string_lossy().to_lowercase() + "\\"))) && !l.contains("\\..\\")
}
pub fn docker_exe() -> PathBuf {
    windows_drive().join(r"Program Files\Docker\Docker\resources\bin\docker.exe")
}
pub fn sys_exe(name: &str) -> PathBuf {
    match name {
        "powershell" => system32().join(r"WindowsPowerShell\v1.0\powershell.exe"),
        "explorer" => system32().with_file_name("explorer.exe"),
        _ => system32().join(format!("{name}.exe")),
    }
}
/// A Windows program, run from System32 so nothing in Lighten's own folder is picked up.
pub fn sys_cmd(name: &str) -> Command {
    let mut c = Command::new(sys_exe(name));
    c.current_dir(system32()).creation_flags(NO_WINDOW);
    c
}

/// Text made safe to put between single quotes. PowerShell also ends a string at curly quotes.
pub fn ps_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}') {
            out.push(c);
        }
        out.push(c);
    }
    out
}

/// Only Windows' own modules load, so a module planted in Documents cannot replace a command.
const PRELUDE: &str = "$env:PSModulePath=\"$PSHOME\\Modules\"\r\n$ProgressPreference='SilentlyContinue'\r\n[Console]::OutputEncoding=[Text.Encoding]::UTF8\r\n";
/// Put first in a script that reads a list: fills $lines from standard input.
pub const READ_LINES: &str = "$lines=@((New-Object IO.StreamReader([Console]::OpenStandardInput(),(New-Object Text.UTF8Encoding $false))).ReadToEnd() -split \"`r?`n\" | Where-Object { $_ })\r\n";

pub fn ps_command(script: &str) -> Command {
    let sys = system32();
    let path = [sys.clone(), sys.with_file_name(""), sys.join("wbem"), sys.join(r"WindowsPowerShell\v1.0")];
    let mut c = sys_cmd("powershell");
    c.env("PATH", std::env::join_paths(path).unwrap_or_default())
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command"])
        .arg(format!("{PRELUDE}{script}"));
    c
}
pub fn run_ps(script: &str) -> Result<String, String> {
    run_ps_input(script, "")
}
/// `input` reaches the script on standard input (see READ_LINES).
pub fn run_ps_input(script: &str, input: &str) -> Result<String, String> {
    let mut child = ps_command(script).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("powershell: {e}"))?;
    let mut pipe = child.stdin.take().ok_or("powershell: no input pipe")?;
    let text = input.to_string();
    let writer = std::thread::spawn(move || pipe.write_all(text.as_bytes()));
    let o = child.wait_with_output().map_err(|e| format!("powershell: {e}"))?;
    let _ = writer.join();
    let mut s = String::from_utf8_lossy(&o.stdout).to_string();
    s.push_str(&String::from_utf8_lossy(&o.stderr));
    Ok(s.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_run_together_keep_their_own_output() {
        let jobs: Vec<_> = (0..6).map(|i| std::thread::spawn(move || (i, run_ps(&format!("Start-Sleep -Milliseconds 200; 'job-{i}'"))))).collect();
        for j in jobs {
            let (i, out) = j.join().unwrap();
            assert_eq!(out.unwrap(), format!("job-{i}"));
        }
    }
    #[test]
    fn lists_arrive_on_standard_input_unchanged() {
        let list = "C:\\a b\\it's.txt\r\nC:\\\u{e9}\u{2019}$(1+1)\\x";
        let out = run_ps_input(&format!("{READ_LINES}$lines | ForEach-Object {{ \"[$_]\" }}"), list).unwrap();
        assert_eq!(out.replace('\r', ""), "[C:\\a b\\it's.txt]\n[C:\\\u{e9}\u{2019}$(1+1)\\x]");
    }
    #[test]
    fn quoted_text_cannot_run_as_code() {
        for nasty in ["a'; 'ran", "a\u{2019}; 'ran", "a\u{2018}; 'ran", "$(1+1) `n \"x\""] {
            assert_eq!(run_ps(&format!("'{}'", ps_quote(nasty))).unwrap(), nasty);
        }
    }
    #[test]
    fn multi_line_scripts_with_quotes_survive() {
        let out = run_ps("$a=@('x \"y\"')\r\nif($a.Count -eq 1){\r\n  \"got $($a[0]) \\ end\"\r\n}").unwrap();
        assert_eq!(out, "got x \"y\" \\ end");
    }
    #[test]
    fn built_in_commands_still_load() {
        let out = run_ps("foreach($c in 'Get-CimInstance','Checkpoint-Computer','Repair-Volume','Clear-RecycleBin','ConvertTo-Json','Add-Type'){ if(-not (Get-Command $c -ErrorAction SilentlyContinue)){ \"missing $c\" } }; 'done'").unwrap();
        assert_eq!(out, "done");
    }
    #[test]
    fn system_tools_are_named_by_full_path() {
        for n in ["powershell", "explorer", "cmd", "reg", "schtasks", "robocopy", "dism"] {
            assert!(sys_exe(n).is_absolute() && sys_exe(n).is_file(), "{n}");
        }
    }
    /// A bare name would run a planted copy sitting beside Lighten.exe.
    #[test]
    fn no_program_is_started_by_bare_name() {
        let bare = concat!("Command::new", "(\"");
        for src in [include_str!("main.rs"), include_str!("tools.rs"), include_str!("extra.rs"), include_str!("more.rs"), include_str!("launch.rs")] {
            assert!(!src.contains(bare));
        }
    }
    #[test]
    fn only_program_files_counts_as_installed() {
        let pf = windows_drive().join("Program Files");
        assert!(in_program_files(&pf.join(r"Lighten\Lighten.exe")));
        assert!(!in_program_files(&windows_drive().join(r"Users\a\Downloads\Lighten.exe")));
        assert!(!in_program_files(&windows_drive().join(r"Program Files Fake\Lighten.exe")));
        assert!(!in_program_files(&pf.join(r"..\Users\a\Lighten.exe")));
    }
}
