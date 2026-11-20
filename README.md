# leaveCalone (Lighten)

Windows disk cleaner. Frees space on C:, moves big folders to another drive behind junctions (copy, verify, link, rollback), finds large and duplicate files, and runs scheduled scans.

- Runs as administrator (one prompt at launch).
- Built with Tauri 2 and Rust. `cargo build --release` produces `target/release/lighten.exe`.
- The exe is unsigned, so Windows SmartScreen may warn.
