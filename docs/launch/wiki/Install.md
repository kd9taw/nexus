# Install & Verify

Everything you need to install Nexus on Windows, verify the download, upgrade,
uninstall, and know where your data lives. If you just want to get on the air,
[Quick Start](Quick-Start) covers install in a few paragraphs; come here for the
complete picture.

[**⬇ Download the latest Windows installer**](https://sourceforge.net/projects/nexus-ham-radio/files/latest/download)

---

## What you need

- **Windows 10 or 11, 64-bit (x64).** Windows is the supported platform today.
  The codebase is cross-platform Rust/Tauri and builds on Linux, but only the
  Windows installer ships.
- **A radio with CAT + audio**, or a network rig (FlexRadio, remote `rigctld`).
  You can install and explore without a radio — the wizard and every panel open —
  but you need a rig connected to transmit.
- **Nothing else to install.** The installer bundles the **WebView2** runtime and
  **Hamlib** (`rigctld.exe` plus its DLLs), so CAT and rotor control work offline
  out of the box. USB bridge-chip drivers are the one exception — if Windows is
  missing one, the first-run wizard flags it with the download link.

The installer is roughly **210 MB** because it carries the WebView2 runtime,
Hamlib, and the DSP stack so a bare PC works with no internet.

---

## Download

The installer is `Nexus_<version>_x64-setup.exe`. Get it from **SourceForge**:

- Latest build: <https://sourceforge.net/projects/nexus-ham-radio/files/latest/download>
- All files (including the SHA-256 for each release):
  <https://sourceforge.net/projects/nexus-ham-radio/files/>

---

## Verify the download

Because the installer is **unsigned** (see below), verifying the checksum is the
way to confirm you have an untampered copy. Each release publishes a `SHA-256`
alongside the `.exe`.

In PowerShell, from the folder where you saved the installer:

```powershell
Get-FileHash .\Nexus_<version>_x64-setup.exe -Algorithm SHA256
```

Compare the printed hash against the value on the download page — they must match
exactly (case doesn't matter). If they differ, delete the file and download again
from the official source above.

---

## Install and the SmartScreen warning

Run the installer. The published binaries are cross-compiled and **unsigned**, so
Windows SmartScreen shows a blue *"Windows protected your PC"* dialog. This is
expected for an unsigned installer and does not indicate a problem with the file —
which is exactly why the SHA-256 check above is worth doing.

Click **More info**, then **Run anyway**.

If you'd rather avoid the prompt entirely, you can
[build from source](https://github.com/kd9taw/nexus) instead.

**Where it installs:** Nexus installs **per-user** — no administrator rights, no
system-wide changes. The program files land under your user profile
(`%LOCALAPPDATA%\Programs\`), and a Start-menu entry is created for your account
only.

---

## Upgrading

There is **no auto-update**. To upgrade, download the newer installer and run it —
it installs over the existing version in place. Your settings and logbook live in
a separate location (below) and are left untouched, so upgrading never disturbs
your data. To confirm you're on the build you expect, check the build hash in the
Settings header against the release you installed.

---

## Uninstalling

Uninstall from **Settings ▸ Apps ▸ Installed apps** (or the Start-menu
uninstaller) like any Windows program. Uninstalling removes the program files but
**leaves your data** — settings and logbook — in place, so reinstalling later
picks up exactly where you left off. For a truly clean removal, delete the data
folders below by hand after uninstalling.

---

## Where your data lives

| What | Location | Notes |
|---|---|---|
| Settings | `%APPDATA%\tempo\settings.json` | JSON, camelCase keys; partial files merge with defaults, so it's safe to hand-edit |
| **Logbook** | `%APPDATA%\tempo\log.adi` | ADIF 3.1.4 — **this is the file to back up** |
| Received-audio recordings | `%APPDATA%\tempo\recordings\` | Only if you enable audio saving; can get large |
| UI state | `%LOCALAPPDATA%\com.kd9taw.tempo\` | Theme, UI scale, panel layout, wizard-seen flag, board filters |

Two things worth understanding:

- **`log.adi` is the irreplaceable file.** Everything else can be rebuilt or
  re-entered; your contacts can't. Back it up. It's plain ADIF, so any logger can
  read it, and Nexus round-trips it faithfully.
- **UI preferences don't roam with settings.** Theme, UI scale, and layout live
  in the WebView2 store under `%LOCALAPPDATA%\com.kd9taw.tempo`, not in
  `settings.json`. Copying `settings.json` to another machine carries your rig and
  station config but not your theme or window layout.

Credentials for online services (LoTW, QRZ, ClubLog, eQSL, HRDLog) are **not** in
any of these files — they live in the Windows Credential Manager (the OS keychain)
and are never written to config or logs.

---

## Backing up

Before a reinstall, a PC migration, or just periodically, copy:

- `%APPDATA%\tempo\log.adi` — your logbook (**the important one**)
- `%APPDATA%\tempo\settings.json` — your rig/station config, to save re-entering it

To restore, install Nexus, then drop those files back into `%APPDATA%\tempo\`
before launching. Online-service credentials will need to be re-entered from
Settings, since they don't leave the origin machine's keychain.

---

## See also

- [Quick Start](Quick-Start) — install to first QSO in 15 minutes.
- [Rig Setup](Rig-Setup) — CAT, PTT, and audio per brand.
- [FAQ](FAQ) — the common questions.
- [Documentation](Documentation) — the full manual set on GitHub.

---

*Nexus is GPL-3.0-or-later. Built by Seth McCallister, KD9TAW.*
