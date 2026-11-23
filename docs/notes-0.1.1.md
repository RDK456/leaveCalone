## Lighten v0.1.1

**New**
- Hidden-space check: Fast scan flags large files the disk lists but Windows cannot find, with a button that schedules a disk check (`chkdsk /f`) for the next restart
- Free-space drop alert: banner on Overview and a notification from the scheduled scan when C: loses 10 GB or more
- System page: clear Windows crash reports, delete old restore points, read-only disk check
- Ask box: type a problem and it opens the page or setting that fixes it (offline, keyword-based)
- Every setting now has a plain explanation
- Similar-photo scan limit is selectable (6,000 to 100,000)
- Undo no longer lists files already emptied from the Recycle Bin
- Command-line output can be redirected to a file

**Known limits**
- Not code-signed, so SmartScreen may warn (More info, Run anyway)
- Little real-world testing of: deletes, OneDrive on cloud files, Undo restore, startup toggle, the new System buttons and the disk-check scheduler. Use the dry-run report first
- The Ask box only understands its keywords

SHA-256 of `Lighten.exe`: `5dbb0beb609928ae6857ef8baf3aae2ff471f63904ba783e244753111649a419`
