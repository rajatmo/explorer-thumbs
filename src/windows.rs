//! The COM side: what File Explorer loads and calls.
//!
//! Explorer's thumbnail host (`dllhost.exe`) loads this DLL, calls [`DllGetClassObject`] for
//! [`CLSID_THUMBNAILS`], creates one [`Provider`] per file, hands it the file as a stream
//! (`IInitializeWithStream`) and asks for a bitmap (`IThumbnailProvider`). Every failure is
//! reported as `WTS_E_FAILEDEXTRACTION`, on which Explorer shows the file's icon.
//!
//! This is the only module of the crate that may use `unsafe`. The PDF is parsed and drawn by
//! safe code ([`crate::first_page`]).

use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_OUTOFMEMORY, E_POINTER, E_UNEXPECTED, ERROR_ALREADY_INITIALIZED, S_FALSE, S_OK,
};
use windows::Win32::Graphics::Gdi::{BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDIBSection, DIB_RGB_COLORS, HBITMAP};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl, IStream, STATFLAG_NONAME, STATSTG};
use windows::Win32::UI::Shell::PropertiesSystem::{IInitializeWithStream, IInitializeWithStream_Impl};
use windows::Win32::UI::Shell::{IThumbnailProvider, IThumbnailProvider_Impl, WTS_ALPHATYPE, WTS_E_FAILEDEXTRACTION, WTSAT_RGB};
use windows_core::{BOOL, Error, GUID, HRESULT, IUnknown, Interface, Ref, Result, implement};

/// This handler's class: `{5FA886BA-5F03-435C-AAE3-6B5FF47E02EA}`. It is this add-on's own, so a
/// thumbnail handler that PdfCraft itself may ship later is a different class and the two cannot
/// overwrite each other's registration. The installer registers exactly this value.
pub const CLSID_THUMBNAILS: GUID = GUID::from_u128(0x5FA886BA_5F03_435C_AAE3_6B5FF47E02EA);

/// Files larger than this get no thumbnail: the whole file is read into the host's memory.
const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;

/// Objects alive plus `LockServer` locks; the DLL may be unloaded at zero.
static IN_USE: AtomicUsize = AtomicUsize::new(0);

/// Counts one user of the DLL for as long as it lives.
struct InUse;

impl InUse {
    fn new() -> Self {
        IN_USE.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for InUse {
    fn drop(&mut self) {
        IN_USE.fetch_sub(1, Ordering::SeqCst);
    }
}

/// One file's thumbnail provider.
#[implement(IInitializeWithStream, IThumbnailProvider)]
struct Provider {
    /// The file, from `Initialize` until `GetThumbnail` reads it.
    stream: Mutex<Option<IStream>>,
    _in_use: InUse,
}

impl IInitializeWithStream_Impl for Provider_Impl {
    fn Initialize(&self, pstream: Ref<'_, IStream>, _grfmode: u32) -> Result<()> {
        let stream = pstream.ok()?.clone();
        let mut held = self.stream.lock().map_err(|_| Error::from(E_UNEXPECTED))?;
        if held.is_some() {
            return Err(ERROR_ALREADY_INITIALIZED.to_hresult().into());
        }
        *held = Some(stream);
        Ok(())
    }
}

impl IThumbnailProvider_Impl for Provider_Impl {
    fn GetThumbnail(&self, cx: u32, phbmp: *mut HBITMAP, pdwalpha: *mut WTS_ALPHATYPE) -> Result<()> {
        if phbmp.is_null() || pdwalpha.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: both pointers were checked for null, and COM requires the caller to pass
        // valid, writable locations for out-parameters.
        unsafe {
            phbmp.write(HBITMAP::default());
            pdwalpha.write(WTSAT_RGB);
        }
        let stream = self.stream.lock().map_err(|_| Error::from(E_UNEXPECTED))?.clone().ok_or(Error::from(E_UNEXPECTED))?;
        let bytes = read_all(&stream).map_err(|()| Error::from(WTS_E_FAILEDEXTRACTION))?;
        let thumb = crate::first_page(bytes, cx).map_err(|_| Error::from(WTS_E_FAILEDEXTRACTION))?;
        let bitmap = to_bitmap(&thumb)?;
        // SAFETY: `phbmp` is non-null and writable (above). The caller owns the bitmap from here.
        unsafe { phbmp.write(bitmap) };
        Ok(())
    }
}

/// The whole stream as bytes, from its start. An error for an empty or oversized file.
fn read_all(stream: &IStream) -> std::result::Result<Vec<u8>, ()> {
    let mut stat = STATSTG::default();
    // SAFETY: `stat` is a valid STATSTG to fill; STATFLAG_NONAME means no name string is
    // allocated, so there is nothing to free.
    unsafe { stream.Stat(&mut stat, STATFLAG_NONAME) }.map_err(|_| ())?;
    if stat.cbSize == 0 || stat.cbSize > MAX_FILE_BYTES {
        return Err(());
    }
    let size = usize::try_from(stat.cbSize).map_err(|_| ())?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(size).map_err(|_| ())?;
    bytes.resize(size, 0);
    let mut filled = 0;
    while filled < size {
        let rest = &mut bytes[filled..];
        let ask = u32::try_from(rest.len()).unwrap_or(u32::MAX);
        let mut got = 0u32;
        // SAFETY: `rest` is a live, writable buffer of at least `ask` bytes, and `got` is a valid
        // place for the count.
        let result = unsafe { stream.Read(rest.as_mut_ptr().cast(), ask, Some(&mut got)) };
        if result.is_err() {
            return Err(());
        }
        if got == 0 {
            break;
        }
        filled = filled.saturating_add(got as usize).min(size);
    }
    bytes.truncate(filled);
    if bytes.is_empty() { Err(()) } else { Ok(bytes) }
}

/// A 32-bit top-down bitmap holding the thumbnail's pixels.
fn to_bitmap(thumb: &crate::Thumbnail) -> Result<HBITMAP> {
    let width = i32::try_from(thumb.width).map_err(|_| Error::from(WTS_E_FAILEDEXTRACTION))?;
    let height = i32::try_from(thumb.height).map_err(|_| Error::from(WTS_E_FAILEDEXTRACTION))?;
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            // Negative: rows run from the top, as in `Thumbnail`.
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut c_void = std::ptr::null_mut();
    // SAFETY: `info` describes a 32-bit uncompressed bitmap and outlives the call; `bits` is a
    // valid place for the pixel pointer; no device context or file mapping is used.
    let bitmap = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) }?;
    if bitmap.is_invalid() || bits.is_null() {
        return Err(E_OUTOFMEMORY.into());
    }
    // SAFETY: CreateDIBSection allocated width × height × 4 bytes at `bits` for this 32-bit
    // bitmap (rows of 32-bit pixels need no padding), which is exactly `thumb.bgra.len()`:
    // `first_page` returns width × height × 4 bytes. The two buffers cannot overlap.
    unsafe { std::ptr::copy_nonoverlapping(thumb.bgra.as_ptr(), bits.cast::<u8>(), thumb.bgra.len()) };
    Ok(bitmap)
}

/// Creates [`Provider`]s.
#[implement(IClassFactory)]
struct Factory {
    _in_use: InUse,
}

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(&self, punkouter: Ref<'_, IUnknown>, riid: *const GUID, ppvobject: *mut *mut c_void) -> Result<()> {
        if riid.is_null() || ppvobject.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: `ppvobject` was checked for null; COM requires it to be writable.
        unsafe { ppvobject.write(std::ptr::null_mut()) };
        if punkouter.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let provider: IUnknown = Provider { stream: Mutex::new(None), _in_use: InUse::new() }.into();
        // SAFETY: `riid` and `ppvobject` are non-null (above) and valid by the COM contract;
        // `query` writes an owned interface pointer or null.
        unsafe { provider.query(riid, ppvobject) }.ok()
    }

    fn LockServer(&self, flock: BOOL) -> Result<()> {
        if flock.as_bool() {
            IN_USE.fetch_add(1, Ordering::SeqCst);
        } else {
            IN_USE.fetch_sub(1, Ordering::SeqCst);
        }
        Ok(())
    }
}

/// COM asks for the class factory of a class this DLL serves.
///
/// # Safety
/// Called by COM: `rclsid` and `riid` point to GUIDs and `ppv` to a writable pointer, or are null.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(rclsid: *const GUID, riid: *const GUID, ppv: *mut *mut c_void) -> HRESULT {
    if rclsid.is_null() || riid.is_null() || ppv.is_null() {
        return E_POINTER;
    }
    // SAFETY: `ppv` is non-null; COM requires it to be writable.
    unsafe { ppv.write(std::ptr::null_mut()) };
    // SAFETY: `rclsid` is non-null and points to a GUID by the COM contract.
    if unsafe { *rclsid } != CLSID_THUMBNAILS {
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    let factory: IUnknown = Factory { _in_use: InUse::new() }.into();
    // SAFETY: `riid` and `ppv` are non-null and valid by the COM contract.
    unsafe { factory.query(riid, ppv) }
}

/// COM asks whether the DLL can be unloaded: yes once no object or lock is left.
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if IN_USE.load(Ordering::SeqCst) == 0 { S_OK } else { S_FALSE }
}

#[cfg(test)]
mod tests {
    use windows::Win32::Graphics::Gdi::{BITMAP, DeleteObject, GetObjectW};
    use windows::Win32::UI::Shell::SHCreateMemStream;

    use super::*;
    use crate::tests::one_page_pdf;

    /// A provider obtained the way Explorer obtains one, holding `file`.
    fn provider_for(file: &[u8]) -> IThumbnailProvider {
        let mut raw: *mut c_void = std::ptr::null_mut();
        // SAFETY: valid GUID pointers and a valid out-pointer.
        let hr = unsafe { DllGetClassObject(&CLSID_THUMBNAILS, &IClassFactory::IID, &mut raw) };
        assert!(hr.is_ok(), "class factory: {hr:?}");
        // SAFETY: on success `raw` is an owned IClassFactory pointer.
        let factory = unsafe { IClassFactory::from_raw(raw) };
        // SAFETY: no aggregation; the interface is requested by type.
        let init: IInitializeWithStream = unsafe { factory.CreateInstance(None) }.expect("provider");
        // SAFETY: the slice is copied into the new stream.
        let stream = unsafe { SHCreateMemStream(Some(file)) }.expect("stream");
        // SAFETY: a valid stream; 0 is STGM_READ.
        unsafe { init.Initialize(&stream, 0) }.expect("initialize");
        // SAFETY: as above.
        let again = unsafe { init.Initialize(&stream, 0) };
        assert!(again.is_err(), "a provider takes one file only");
        init.cast().expect("IThumbnailProvider")
    }

    #[test]
    fn explorer_gets_a_bitmap_of_the_first_page_that_fits_the_size_it_asked_for() {
        let provider = provider_for(&one_page_pdf(600, 300, "1 0 0 rg 0 0 600 300 re f"));
        let (mut bitmap, mut alpha) = (HBITMAP::default(), WTS_ALPHATYPE::default());
        // SAFETY: both out-parameters are valid; the bitmap returned is ours.
        unsafe { provider.GetThumbnail(256, &mut bitmap, &mut alpha) }.expect("thumbnail");
        let mut seen = BITMAP::default();
        // SAFETY: `seen` is a BITMAP of the size passed.
        let copied = unsafe { GetObjectW(bitmap.into(), size_of::<BITMAP>() as i32, Some((&raw mut seen).cast())) };
        assert!(copied > 0, "the handle is a bitmap");
        assert!(seen.bmWidth <= 256 && seen.bmHeight <= 256, "{}×{} fits 256", seen.bmWidth, seen.bmHeight);
        assert!(seen.bmWidth > seen.bmHeight, "a landscape page stays landscape");
        assert_eq!(seen.bmBitsPixel, 32);
        assert_eq!(alpha, WTSAT_RGB, "no transparency: the page is on white paper");
        // SAFETY: the pixel pointer of a DIB section is valid for the bitmap's bytes.
        let first = unsafe { std::slice::from_raw_parts(seen.bmBits.cast::<u8>(), 4) };
        assert!(first[2] > 200 && first[0] < 50, "the red page reached the bitmap as B, G, R, A: {first:?}");
        // SAFETY: the bitmap is ours to free.
        let _ = unsafe { DeleteObject(bitmap.into()) };
    }

    #[test]
    fn a_file_that_is_not_a_pdf_fails_the_way_that_makes_explorer_show_the_icon() {
        let provider = provider_for(b"This is not a PDF file.");
        let (mut bitmap, mut alpha) = (HBITMAP::default(), WTS_ALPHATYPE::default());
        // SAFETY: both out-parameters are valid.
        let error = unsafe { provider.GetThumbnail(256, &mut bitmap, &mut alpha) }.expect_err("no thumbnail");
        assert_eq!(error.code(), WTS_E_FAILEDEXTRACTION);
        assert!(bitmap.is_invalid(), "no bitmap is handed out on failure");
    }

    #[test]
    fn other_classes_and_missing_pointers_are_refused() {
        let mut raw: *mut c_void = std::ptr::null_mut();
        let other = GUID::from_u128(0x0123_4567_89AB_CDEF_0123_4567_89AB_CDEF);
        // SAFETY: valid pointers.
        assert_eq!(unsafe { DllGetClassObject(&other, &IClassFactory::IID, &mut raw) }, CLASS_E_CLASSNOTAVAILABLE);
        assert!(raw.is_null());
        // SAFETY: null is checked for before anything is read or written.
        assert_eq!(unsafe { DllGetClassObject(&CLSID_THUMBNAILS, &IClassFactory::IID, std::ptr::null_mut()) }, E_POINTER);
    }
}
