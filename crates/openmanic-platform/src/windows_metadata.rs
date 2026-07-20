//! Worker-only Windows executable icon extraction.

use std::mem::size_of;

use openmanic_application::{
    ApplicationIcon, ApplicationIconDigest, ApplicationIconKey, ApplicationIconResult,
};
use windows::{
    Win32::{
        Graphics::Gdi::{
            BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDIBits,
            GetObjectW, HBITMAP, HGDIOBJ,
        },
        Storage::FileSystem::FILE_ATTRIBUTE_NORMAL,
        UI::{
            Shell::{SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGetFileInfoW},
            WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO},
        },
    },
    core::PCWSTR,
};

use crate::WindowsApplicationMetadataRequest;

/// Extracts a decoded executable icon, returning an ordinary fallback when Windows cannot supply
/// a usable icon. This function is intentionally called only from the metadata worker.
#[must_use]
pub fn extract_application_icon(
    request: &WindowsApplicationMetadataRequest,
) -> ApplicationIconResult {
    decode_executable_icon(request.executable_path()).map_or(
        ApplicationIconResult::Fallback {
            application_id: request.application_id(),
        },
        |icon| ApplicationIconResult::Decoded {
            key: ApplicationIconKey::new(
                request.application_id(),
                ApplicationIconDigest::from_bytes(icon_digest(&icon)),
            ),
            icon,
        },
    )
}

fn decode_executable_icon(path: &str) -> Option<ApplicationIcon> {
    let mut wide_path: Vec<u16> = path.encode_utf16().collect();
    wide_path.push(0);
    let mut file_info = SHFILEINFOW::default();
    // SAFETY: `wide_path` remains NUL-terminated and alive throughout the call, and Windows only
    // initializes the writable `SHFILEINFOW` supplied for the documented structure size.
    let found = unsafe {
        SHGetFileInfoW(
            PCWSTR(wide_path.as_ptr()),
            FILE_ATTRIBUTE_NORMAL,
            Some(&raw mut file_info),
            u32::try_from(size_of::<SHFILEINFOW>()).ok()?,
            SHGFI_ICON | SHGFI_LARGEICON,
        )
    };
    if found == 0 || file_info.hIcon.0.is_null() {
        return None;
    }
    let icon = OwnedIcon(file_info.hIcon);
    decode_icon(icon.0)
}

fn decode_icon(icon: HICON) -> Option<ApplicationIcon> {
    let mut info = ICONINFO::default();
    // SAFETY: `info` is a valid writable ICONINFO. The icon is owned for this call and Windows
    // returns independent bitmap handles that `OwnedBitmap` releases below.
    unsafe { GetIconInfo(icon, &raw mut info).ok()? };
    let color = OwnedBitmap(info.hbmColor);
    let _mask = OwnedBitmap(info.hbmMask);
    if color.0.0.is_null() {
        return None;
    }

    let mut bitmap = BITMAP::default();
    // SAFETY: `bitmap` has the exact documented size and `color` remains valid during the call.
    let copied = unsafe {
        GetObjectW(
            HGDIOBJ(color.0.0),
            i32::try_from(size_of::<BITMAP>()).ok()?,
            Some((&raw mut bitmap).cast()),
        )
    };
    if copied == 0 || bitmap.bmWidth <= 0 || bitmap.bmHeight <= 0 {
        return None;
    }
    let width = u32::try_from(bitmap.bmWidth).ok()?;
    let height = u32::try_from(bitmap.bmHeight).ok()?;
    let byte_len = usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?
        .checked_mul(4)?;
    let mut bgra = vec![0_u8; byte_len];
    let mut bitmap_info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: u32::try_from(size_of::<BITMAPINFOHEADER>()).ok()?,
            biWidth: bitmap.bmWidth,
            biHeight: -bitmap.bmHeight,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..BITMAPINFOHEADER::default()
        },
        ..BITMAPINFO::default()
    };
    // SAFETY: `bgra` has room for exactly width*height 32-bit pixels, and `bitmap_info` requests
    // a top-down 32-bit DIB. The compatible DC is optional for `GetDIBits`, so a null DC is safe.
    let lines = unsafe {
        GetDIBits(
            windows::Win32::Graphics::Gdi::HDC::default(),
            color.0,
            0,
            height,
            Some(bgra.as_mut_ptr().cast()),
            &raw mut bitmap_info,
            DIB_RGB_COLORS,
        )
    };
    if lines != i32::try_from(height).ok()? {
        return None;
    }
    let rgba = bgra_to_rgba(bgra);
    ApplicationIcon::try_new(width, height, rgba).ok()
}

fn bgra_to_rgba(mut pixels: Vec<u8>) -> Vec<u8> {
    let alpha_is_empty = pixels.chunks_exact(4).all(|pixel| pixel[3] == 0);
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        if alpha_is_empty {
            pixel[3] = u8::MAX;
        }
    }
    pixels
}

fn icon_digest(icon: &ApplicationIcon) -> [u8; 32] {
    let mut state = [0x9e37_79b9_u32, 0x85eb_ca6b, 0xc2b2_ae35, 0x27d4_eb2f];
    for byte in icon
        .width()
        .to_le_bytes()
        .into_iter()
        .chain(icon.height().to_le_bytes())
        .chain(icon.rgba().iter().copied())
    {
        for lane in &mut state {
            *lane = lane.rotate_left(5) ^ u32::from(byte);
            *lane = lane.wrapping_mul(0x9e37_79b1);
        }
    }
    let mut digest = [0_u8; 32];
    for (index, lane) in state.iter().enumerate() {
        for (offset, byte) in lane.to_le_bytes().iter().enumerate() {
            digest[index * 4 + offset] = *byte;
            digest[16 + index * 4 + offset] = byte.rotate_left(1) ^ 0xa5;
        }
    }
    digest
}

struct OwnedIcon(HICON);

impl Drop for OwnedIcon {
    fn drop(&mut self) {
        // SAFETY: This wrapper owns the HICON returned by SHGetFileInfoW and destroys it once.
        let _ = unsafe { DestroyIcon(self.0) };
    }
}

struct OwnedBitmap(HBITMAP);

impl Drop for OwnedBitmap {
    fn drop(&mut self) {
        if !self.0.0.is_null() {
            // SAFETY: This wrapper owns the bitmap handles returned by GetIconInfo and deletes
            // each one once after all reads have completed.
            let _ = unsafe { DeleteObject(HGDIOBJ(self.0.0)) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{bgra_to_rgba, icon_digest};
    use openmanic_application::ApplicationIcon;

    #[test]
    fn converts_bgra_and_normalizes_legacy_empty_alpha() {
        assert_eq!(bgra_to_rgba(vec![3, 2, 1, 0]), vec![1, 2, 3, 255]);
        assert_eq!(bgra_to_rgba(vec![3, 2, 1, 9]), vec![1, 2, 3, 9]);
    }

    #[test]
    fn digest_is_deterministic_and_covers_dimensions() {
        let one = ApplicationIcon::try_new(1, 1, vec![1, 2, 3, 4]).expect("fixture icon");
        let two =
            ApplicationIcon::try_new(2, 1, vec![1, 2, 3, 4, 0, 0, 0, 0]).expect("fixture icon");
        assert_eq!(icon_digest(&one), icon_digest(&one));
        assert_ne!(icon_digest(&one), icon_digest(&two));
    }
}
