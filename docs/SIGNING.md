# Code signing

`Lighten.exe` is not code-signed, so Windows SmartScreen shows "unknown publisher" until the file builds reputation or is signed. There are two separate signatures, and only the first is in place:

| Signature | What it protects | Status |
| --- | --- | --- |
| ed25519 update signature | In-app updates: a manifest is refused unless it was signed with the private key. Public key: `update.pub`. | Done |
| Authenticode (code signing) | The SmartScreen "unknown publisher" warning on first run. | Not done |

## Getting an Authenticode signature

A certificate has to be issued to a verified person or organization, so this cannot be generated from the repo. Options:

- **SignPath Foundation**: free signing for open-source projects. Needs a public repo with an OSI-approved license (this repo is MIT) and an application. See their [terms](https://signpath.org/terms).
- **Azure Trusted Signing**: pay-as-you-go, needs an Azure account and identity validation.
- **A commercial certificate** (OV or EV).

No option removes the SmartScreen warning at once: reputation builds over releases signed by the same publisher ([Microsoft](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options)).

Once you have one, sign the release build before uploading it:

```powershell
signtool sign /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 /a target\release\lighten.exe
```

The release workflow (`.github/workflows/release.yml`) does this automatically once the `AUTHENTICODE_PFX_BASE64` and `AUTHENTICODE_PFX_PASSWORD` repository secrets exist. It only works with a certificate you can export as a `.pfx`; SignPath and Azure Trusted Signing need their own signing step instead.

Re-sign after every build. The delta updater's SHA-256 and ed25519 signature must be generated from the signed file, because signing changes the bytes.
