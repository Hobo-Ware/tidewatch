# Packaging Tidewatch

Tidewatch ships as a universal macOS app (Homebrew cask), a Linux tarball and an AppImage.
`tidewatch.rb` is a reference copy of the cask: the release workflow only swaps its `version`
and `sha256` lines.

## Release flow

1. Bump `version` in the workspace `Cargo.toml` and update `LEDGER.md`.
2. Tag and push: `git tag v0.2.0 && git push origin v0.2.0` (or run the `release` workflow by hand with the tag).
3. `.github/workflows/release.yml` checks the tag matches `Cargo.toml`, then builds in parallel:
   - macOS: tests, arm64 and x86_64 binaries, `scripts/bundle-macos.sh` (lipo, `Tidewatch.app`, ad-hoc signature, `ditto` zip).
   - Linux: `scripts/package-linux.sh` (tarball, plus an AppImage through the pinned `appimagetool`).
4. The release job writes `SHA256SUMS` and the filled-in cask, and creates the GitHub Release with everything.
5. The tap job commits `Casks/tidewatch.rb` to `Hobo-Ware/homebrew-tap`, but only when the repository
   variable `PUBLISH_TAP` is `true`. Leave it unset while the repo is private. A re-run for an older tag never rolls the tap back.

## One-time setup (when the repo goes public)

```sh
ssh-keygen -t ed25519 -N "" -C "tidewatch release -> homebrew-tap" -f tap_key
gh repo deploy-key add tap_key.pub -R Hobo-Ware/homebrew-tap -t "tidewatch release" -w
gh secret set HOMEBREW_TAP_DEPLOY_KEY -R Hobo-Ware/tidewatch < tap_key
rm tap_key tap_key.pub
gh variable set PUBLISH_TAP --body true -R Hobo-Ware/tidewatch
```

## Local builds

```sh
scripts/bundle-macos.sh     # target/dist/Tidewatch.app and tidewatch-<version>-universal.app.zip
scripts/package-linux.sh    # x86_64 Linux only: target/dist/ tarball, and an AppImage if appimagetool is found
```

`SIGN_IDENTITY` replaces the ad-hoc signature (`-`) with a real identity. `APPIMAGETOOL` points at the tool.

## Signing

Builds are ad-hoc signed and the cask strips `com.apple.quarantine` after install so Gatekeeper
does not block the launch. Developer ID signing and notarization wait for an Apple Developer account (PLAN.md D10).

## Icon

`assets/icon.svg` is the app icon (the K mark); `assets/AppIcon.icon` is the same mark as Liquid Glass layers, which `make-icon.sh` compiles to `assets/Assets.car` with Xcode 26 `actool` (macOS 26 reads it through `CFBundleIconName`; older macOS falls back to the `.icns`). `scripts/make-icon.sh` renders it into `assets/Tidewatch.icns` and
`assets/icon-512.png` (inkscape, pngquant, oxipng, iconutil). Rerun it after changing the SVG; CI uses the committed files.
