# opensidian on Windows

An unofficial port of [opensidian](https://github.com/jpzk/opensidian) to Windows 11 (x86_64, MSVC). The Linux code paths are unchanged; everything Windows-specific sits behind `cfg(windows)`. What was audited, changed and verified, step by step, is in [AUDIT.md](AUDIT.md).

## Install

Take `opensidian_<version>_x64-setup.exe` from the `opensidian-windows-x86_64-unsigned` artifact of the latest **Build** workflow run, and check it against the `SHA256SUMS` beside it:

```powershell
(Get-FileHash .\opensidian_0.9.2_x64-setup.exe -Algorithm SHA256).Hash.ToLower()
```

- The installer is **not signed**: SmartScreen shows a warning ("More info" > "Run anyway").
- It installs for the current user only, into `%LOCALAPPDATA%\opensidian`, with a Start menu and a desktop shortcut. No administrator rights.
- It needs the WebView2 runtime, which Windows 11 ships. If it is missing, the installer downloads Microsoft's bootstrapper.
- Uninstall from *Settings > Apps*. Your vaults are never touched. The settings file `%USERPROFILE%\.opensidian.json` and the app's data folder `%LOCALAPPDATA%\dev.koto.opensidian` are left in place.

## Build from source

Prerequisites: Visual Studio Build Tools ("Desktop development with C++" with a Windows 11 SDK) and Rust 1.98.0 for `x86_64-pc-windows-msvc`. No Node, no npm.

```powershell
rustup toolchain install 1.98.0 --profile minimal
cd src-tauri
cargo +1.98.0 build --release --locked
cargo +1.98.0 test --locked
```

The binary is `target\release\opensidian.exe` and runs as it is.

The installer needs the Tauri CLI, pinned to the tauri version in `Cargo.lock`:

```powershell
cargo install tauri-cli --locked --version "=2.11.5"
cargo tauri bundle --bundles nsis --config ../packaging/windows/tauri.bundle.conf.json
```

It lands in `target\release\bundle\nsis\`. `tauri bundle` packages the binary that is already built; it does not compile again.

Two things that bite on a developer machine:

- **OneDrive**: do not build inside a synced folder. Set `CARGO_TARGET_DIR` to a local path (`C:\cargo-target\opensidian`), or clone elsewhere.
- **Antivirus**: a scanner that opens freshly linked `.exe` files makes the link step fail now and then (`LNK1104`). Run the command again, or exclude the target directory.

Clone with any `core.autocrlf` setting: `.gitattributes` keeps sources in LF and never converts fixtures or pinned data.

## What differs from Linux

| | Windows |
|---|---|
| Landlock sandbox (`OPENSIDIAN_LANDLOCK=1`) | **Not available.** The process is not confined to the vault. |
| "Never write through a link" | Kept, by other means: every directory on the way is pinned by an open handle and a junction or symlink is refused. A leaf is still checked and then acted on in two steps; see the header of `src-tauri/src/vaultfs_win.rs`. |
| Plugin `fs.chmod` | Only the owner-write bit, mapped to the read-only attribute. |
| Signals to plugin child processes | Every signal ends the process tree. |
| Plugin secrets | Windows Credential Manager, at most 2560 bytes per secret. |
| Plugin store, update check | WinHTTP (Schannel, system trust store and proxy). The update check reports a newer release; there is no Windows asset upstream to point at. |
| Case | `Note.md` and `note.md` cannot coexist in one folder. A case-only rename works. |
| Names | `CON`, `NUL`, `COM1`… , `< > " \| ? *`, and a trailing dot or space are refused. |
| Flatpak, AppImage, deb, rpm, Nix | Not applicable. |

Plugins still see a Linux-like environment (`process.platform === "linux"`, the vault at `/vault`). `child_process.exec()` runs in `cmd.exe`. A plugin that starts a Linux-only program will fail.

## Not verified

- A real community plugin (Dataview, Excalidraw, obsidian-git). The plugin bridge was exercised end to end with a self-written probe plugin only.
- Pasting an image, PDF pages, canvas media cards.
- "Open in new window" and "switch vault" across two live windows.
- A machine without WebView2, and a display scale other than the one it was developed on.
- The `listtoggle` editor tests run on Linux only (they need JavaScriptCore).
