# Code signing

`Lighten.exe` is not code-signed, so Windows SmartScreen shows "unknown publisher" until the file builds reputation or is signed. There are two separate signatures, and only the first is in place:

| Signature | What it protects | Status |
| --- | --- | --- |
| ed25519 update signature | In-app updates: a manifest is refused unless it was signed with the private key. Public key: `update.pub`. | Done |
| Authenticode (code signing) | The SmartScreen "unknown publisher" warning on first run. | Not done |

## Getting an Authenticode signature

A certificate has to be issued to a verified person or organization, so this cannot be generated from the repo. Options:

- **SignPath Foundation**: free signing for open-source projects. Needs a public repo with an OSI-approved license (this repo has no license file yet) and an application.
- **Azure Trusted Signing**: pay-as-you-go, needs an Azure account and identity validation.
- **A commercial certificate** (OV or EV): EV removes the SmartScreen warning at once; OV builds reputation over time.

Once you have one, sign the release build before uploading it:

```powershell
signtool sign /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 /a target\release\lighten.exe
```

Re-sign after every build. The delta updater's SHA-256 and ed25519 signature must be generated from the signed file, because signing changes the bytes.
