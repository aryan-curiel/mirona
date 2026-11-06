# Releasing (NGR-175)

A tag `vX.Y.Z` runs `.github/workflows/release.yml`: it builds the macOS app (Apple silicon,
macOS 14.6+), signs it with the Developer ID certificate and the hardened runtime
(`app/src-tauri/Entitlements.plist`: microphone and calendar), notarizes and staples it, and
attaches the `.app` and `.dmg` to a **draft** GitHub release.

## Cut a release

1. Bump the version in `app/src-tauri/tauri.conf.json`, `app/package.json` and the root
   `Cargo.toml` (the workflow fails if they differ from the tag), then commit.
2. `git tag v0.1.0 && git push origin v0.1.0`
3. When the workflow is green, open the draft release, check the notes, publish.

## One-time setup: repository secrets

| Secret | Value |
| --- | --- |
| `APPLE_CERTIFICATE` | The Developer ID Application certificate and key as a `.p12`, base64 |
| `APPLE_CERTIFICATE_PASSWORD` | The password chosen when exporting the `.p12` |
| `APPLE_SIGNING_IDENTITY` | The certificate's full name, `Developer ID Application: <Name> (<Team ID>)`. `security find-identity -v -p codesigning` lists it |
| `APPLE_ID` | The Apple ID email of the developer account |
| `APPLE_PASSWORD` | An app-specific password for that Apple ID (not the account password) |
| `APPLE_TEAM_ID` | The 10-character Team ID shown in the certificate name |
| `TAURI_SIGNING_PRIVATE_KEY` | The updater signing key (`~/.tauri/mirona-updater.key`, the file's contents) |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Its password; empty for a key made with `--ci` |

- **Certificate:** Keychain Access › login › My Certificates, right-click
  "Developer ID Application: …" › Export as `.p12` with a password, then
  `base64 -i cert.p12 | gh secret set APPLE_CERTIFICATE` and delete the `.p12`.
- **App-specific password:** account.apple.com › Sign-In and Security › App-Specific Passwords.
- **Updater key:** `pnpm tauri signer generate -w ~/.tauri/mirona-updater.key` (from `app/`),
  then `gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.tauri/mirona-updater.key` and the password.
  Its public half is `plugins.updater.pubkey` in `app/src-tauri/tauri.conf.json`. Losing the key
  means installed apps can't update any more: keep a copy somewhere safe.

## Updates

The release build also makes a signed update bundle (`Mirona.app.tar.gz` + `.sig`) and
`latest.json`, attached to the release. Apps that check for updates (Settings › About, off by
default) read `releases/latest/download/latest.json`, so a release reaches them only once it's
published, not while it's a draft. The signature carries the version (`requireSignedVersion`),
so an old bundle can't be served as a new one.

## Local signed build

```bash
cd app
APPLE_SIGNING_IDENTITY="Developer ID Application: <Name> (<Team ID>)" \
  pnpm tauri build --bundles app,dmg
```

Add `APPLE_ID`, `APPLE_PASSWORD` and `APPLE_TEAM_ID` to notarize too. Without a certificate the
build is unsigned and only runs on the machine that built it.

## Not yet

- Windows and Linux builds (NGR-172, NGR-173).
