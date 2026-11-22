# pdfcraft-explorer-thumbs

A local add-on: Windows File Explorer shows the **first page of each PDF as its thumbnail**.

It is not part of PdfCraft's releases. It is a repository of its own that sits inside a PdfCraft
checkout, at `crates/explorer-thumbs`, because it is built against PdfCraft's renderer. It is
installed by its own MSI.

## Setting up

```
git clone https://github.com/storytold/pdfcraft.git
git clone <this repository> pdfcraft/crates/explorer-thumbs
```

PdfCraft's workspace takes every directory under `crates/` as a member, so nothing in PdfCraft is
edited. To keep `git status` in the PdfCraft checkout quiet, add `crates/explorer-thumbs/` to its
`.git/info/exclude`.

## Build and install

From the PdfCraft checkout's root, on Windows x64:

```
powershell -ExecutionPolicy Bypass -File crates\explorer-thumbs\installer\package.ps1
```

It builds the DLL, then the MSI from `installer\thumbs.wxs`, and prints the MSI's path:
`target\explorer-thumbs\PdfCraft-Explorer-Thumbnails-<version>-x64.msi`. It needs WiX v5, the
version PdfCraft's own installer is built with: `dotnet tool install --global wix --version 5.0.2`.

Run the MSI and approve the administrator prompt. It has no pages to click through. Then restart
File Explorer (or sign out and in). Thumbnails Explorer cached earlier stay until its cache is
cleared: Disk Cleanup ▸ Thumbnails.

To remove it: Settings ▸ Apps ▸ Installed apps ▸ "PdfCraft Explorer Thumbnails (local build)".

To see a thumbnail without Explorer:

```
cargo run --release -p pdfcraft-explorer-thumbs --example thumb -- in.pdf out.bmp 256
```

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

## Moving to a newer PdfCraft

This repository holds only the add-on. In the PdfCraft checkout the one trace of it is this
crate's entry in `Cargo.lock`, which cargo writes on every build and which is never committed.
From the PdfCraft checkout's root:

```
git checkout Cargo.lock
git pull
cargo test -p pdfcraft-explorer-thumbs
```

Then run `package.ps1` and the MSI as above: a new build replaces the installed one.

The crate uses three calls of `pdfcraft-render`: `PageRenderer::new`, `page_count` and `render`. If
a future PdfCraft changes them, the build fails in `src/lib.rs` and the tests say whether
thumbnails still come out right.

## Limits

- No thumbnail, and the file's icon instead, for: password-protected PDFs, files over 512 MiB,
  and files the renderer cannot open.
- The MSI and the DLL are not code-signed.
- `cargo xtask layers` reports this crate as unknown: registering it means editing
  `xtask/src/layers.rs`, which this add-on avoids so that the PdfCraft checkout stays unedited.
- `src/windows.rs` uses `unsafe`: a thumbnail handler is an in-process COM server. The workspace
  forbids `unsafe`, so this crate has its own lint table, with `unsafe` denied outside that file.

## Licence

MIT OR Apache-2.0, as PdfCraft.
