//! Extract icons from Windows executables and save as BMP files.

use std::path::Path;
use crate::models::ICON_DIR;

/// Extract the icon from an exe and save it as a BMP file.
/// Returns the path to the saved icon file, or None if extraction fails.
pub fn extract_icon(exe_path: &str) -> Option<String> {
    let _ = std::fs::create_dir_all(ICON_DIR);

    let hash = simple_hash(exe_path);
    let icon_path = format!("{}\\{}.bmp", ICON_DIR, hash);

    // If already extracted, return cached
    if Path::new(&icon_path).is_file() {
        return Some(icon_path);
    }

    unsafe { extract_with_shgetfileinfo(exe_path, &icon_path) }
}

fn simple_hash(s: &str) -> String {
    let mut h: u64 = 5381;
    for b in s.bytes() {
        h = h.wrapping_mul(33).wrapping_add(b as u64);
    }
    format!("{:016x}", h)
}

unsafe fn extract_with_shgetfileinfo(exe_path: &str, output_path: &str) -> Option<String> {
    use windows::Win32::UI::Shell::{SHGetFileInfoA, SHFILEINFOA, SHGFI_ICON, SHGFI_LARGEICON};
    use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, DrawIconEx, DI_NORMAL, ICONINFO};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, SelectObject, GetDIBits, DeleteDC, DeleteObject,
        CreateCompatibleBitmap, GetDC, ReleaseDC, CreateSolidBrush, FillRect,
        BITMAPINFOHEADER, BITMAPINFO, BI_RGB, DIB_RGB_COLORS,
    };
    use windows::Win32::Foundation::COLORREF;

    let exe_cstr = format!("{}\0", exe_path);

    let mut info = SHFILEINFOA::default();
    let result = SHGetFileInfoA(
        windows::core::PCSTR(exe_cstr.as_ptr()),
        windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL,
        Some(&mut info),
        std::mem::size_of::<SHFILEINFOA>() as u32,
        SHGFI_ICON | SHGFI_LARGEICON,
    );

    if result == 0 || info.hIcon.is_invalid() {
        return None;
    }

    let icon = info.hIcon;
    let size: i32 = 32;

    // Create DC and bitmap
    let screen_dc = GetDC(None);
    let mem_dc = CreateCompatibleDC(screen_dc);
    let bitmap = CreateCompatibleBitmap(screen_dc, size, size);
    let old_bmp = SelectObject(mem_dc, bitmap);

    // White background
    let brush = CreateSolidBrush(COLORREF(0x00FFFFFF));
    let rect = windows::Win32::Foundation::RECT { left: 0, top: 0, right: size, bottom: size };
    FillRect(mem_dc, &rect, brush);
    let _ = DeleteObject(brush);

    // Draw icon
    let _ = DrawIconEx(mem_dc, 0, 0, icon, size, size, 0, None, DI_NORMAL);

    // Extract pixels
    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: -size, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0 as u32,
            ..Default::default()
        },
        ..Default::default()
    };

    let row_size = (size as usize) * 4;
    let pixel_data_size = row_size * (size as usize);
    let mut pixels = vec![0u8; pixel_data_size];

    GetDIBits(mem_dc, bitmap, 0, size as u32, Some(pixels.as_mut_ptr() as *mut _), &mut bmi, DIB_RGB_COLORS);

    // Cleanup GDI
    SelectObject(mem_dc, old_bmp);
    let _ = DeleteObject(bitmap);
    let _ = DeleteDC(mem_dc);
    ReleaseDC(None, screen_dc);
    let _ = DestroyIcon(icon);

    // Write BMP
    let file_header_size = 14u32;
    let info_header_size = 40u32;
    let pixel_offset = file_header_size + info_header_size;
    let file_size = pixel_offset + pixel_data_size as u32;

    let mut bmp = Vec::with_capacity(file_size as usize);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&file_size.to_le_bytes());
    bmp.extend_from_slice(&0u16.to_le_bytes());
    bmp.extend_from_slice(&0u16.to_le_bytes());
    bmp.extend_from_slice(&pixel_offset.to_le_bytes());
    bmp.extend_from_slice(&info_header_size.to_le_bytes());
    bmp.extend_from_slice(&size.to_le_bytes());
    bmp.extend_from_slice(&size.to_le_bytes());
    bmp.extend_from_slice(&1u16.to_le_bytes());
    bmp.extend_from_slice(&32u16.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&(pixel_data_size as u32).to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    // Pixel data (flip vertically for BMP bottom-up)
    for y in (0..size as usize).rev() {
        bmp.extend_from_slice(&pixels[y * row_size..(y + 1) * row_size]);
    }

    if std::fs::write(output_path, &bmp).is_ok() {
        Some(output_path.to_string())
    } else {
        None
    }
}
