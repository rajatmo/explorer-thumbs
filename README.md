# pdfcraft-explorer-thumbs

An add-on: Windows File Explorer shows the **first page of each PDF as its thumbnail**.

It is not part of PdfCraft's releases. It is a repository of its own, built against a PdfCraft
checkout that sits beside it, and installed by its own MSI.

## Layout

```
<working folder>\
  pdfcraft\          a plain checkout of storytold/pdfcraft; nothing in it is edited
  explorer-thumbs\   this repository
```

```
git clone https://github.com/storytold/pdfcraft.git
git clone <this repository> explorer-thumbs
```

## Build the installer

From this repository, on Windows x64:

```
powershell -ExecutionPolicy Bypass -File installer\package.ps1
```

It prints the MSI's path: `dist\PdfCraft-Explorer-Thumbnails-<version>-x64.msi`.

| Option | What it does |
|---|---|
| `-PdfCraft <path>` | The PdfCraft checkout to build against. Default: `..\pdfcraft`. |
| `-Test` | Also runs the format check, clippy and the tests first, and stops if they fail. |
| `-SkipBuild` | Packages the DLL already in `target\release`. |

It needs Rust (MSVC toolchain) and WiX v5, the version PdfCraft's own installer is built with:
`dotnet tool install --global wix --version 5.0.2`.

Run the MSI and approve the administrator prompt. It has no pages to click through. Then restart
File Explorer (or sign out and in). Thumbnails Explorer cached earlier stay until its cache is
cleared: Disk Cleanup ▸ Thumbnails.

To remove it: Settings ▸ Apps ▸ Installed apps ▸ "PdfCraft Explorer Thumbnails (local build)".
The entry's comment names the PdfCraft version and commit the renderer inside the DLL comes from.

## Moving to a newer PdfCraft

```
git -C ..\pdfcraft pull
powershell -ExecutionPolicy Bypass -File installer\package.ps1 -Test
```

Then run the new MSI: it replaces the installed build.

The crate uses three calls of PdfCraft's renderer (`pdfcraft-render`): `PageRenderer::new`,
`page_count` and `render`. If a newer PdfCraft changes them, the build fails in `src\lib.rs`, and
`-Test` says whether thumbnails still come out right. The installed DLL is unaffected either way.

## How the build works

The crate cannot be compiled on its own: it needs the same dependency versions and patches as
PdfCraft's renderer, so it is compiled as a member of PdfCraft's workspace. For the length of the
build, `package.ps1` links this repository into the checkout as `crates\explorer-thumbs` (a
directory junction; no administrator rights). PdfCraft's workspace takes every directory under
`crates\` as a member. Afterwards the script removes the link and puts `Cargo.lock` back, so the
checkout is left exactly as it was. Build output goes to `target\` here, not into the checkout.

If a build is killed half-way, the link can be left behind. Remove it with
`cmd /c rmdir ..\pdfcraft\crates\explorer-thumbs` (this removes the link only) and restore the
lock file with `git -C ..\pdfcraft checkout Cargo.lock`.

## What is installed

| What | Where |
|---|---|
| `pdfcraft_explorer_thumbs.dll` | `C:\Program Files\PdfCraft Explorer Thumbnails\` |
| The COM class `{5FA886BA-5F03-435C-AAE3-6B5FF47E02EA}` | `HKLM\Software\Classes\CLSID` |
| The thumbnail handler of `.pdf` | `HKLM\Software\Classes\.pdf\ShellEx\{e357fccd-…}` |
| The same, system-wide fallback | `HKLM\Software\Classes\SystemFileAssociations\.pdf\ShellEx\{e357fccd-…}` |
| The same, while PdfCraft is the default PDF app | `HKLM\Software\Classes\PdfCraft.Document\ShellEx\{e357fccd-…}` |

It is installed for all users, because Explorer's thumbnail host does not find classes registered
for one user only. It works whichever app is the default for PDFs, as long as that app has no
thumbnail handler of its own.

## Why a PdfCraft update does not break it

- **The DLL contains the renderer.** It does not call the installed PdfCraft, so PdfCraft's version,
  or its absence, makes no difference to it.
- **Nothing is shared with PdfCraft's installer.** A separate MSI product with its own upgrade code,
  its own folder and its own COM class. PdfCraft's installer removes and writes only what it owns.
- **Two of the three registrations are under keys PdfCraft's installer never writes** (`.pdf\ShellEx`
  and `SystemFileAssociations\.pdf\ShellEx`). If an update did drop the `PdfCraft.Document` one,
  Explorer falls back to these. Running this MSI again and choosing Repair restores all three.
- **If PdfCraft ships its own thumbnail handler one day**, it is a different class. Both can be
  installed; this one can then be removed.

## Limits

- No thumbnail, and the file's icon instead, for: password-protected PDFs, files over 512 MiB,
  and files the renderer cannot open.
- The MSI and the DLL are not code-signed.
- An editor opened on this repository alone cannot resolve the crate (see "How the build works").
- `src\windows.rs` uses `unsafe`: a thumbnail handler is an in-process COM server. PdfCraft's
  workspace forbids `unsafe`, so this crate has its own lint table, with `unsafe` denied outside
  that file.

## Licence

MIT OR Apache-2.0, as PdfCraft.
