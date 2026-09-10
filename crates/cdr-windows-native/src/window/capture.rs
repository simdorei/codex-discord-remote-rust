use std::mem::size_of;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, CreateCompatibleBitmap, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
    DeleteObject, GetDC, GetDIBits, HBITMAP, HDC, HGDIOBJ, ReleaseDC, SelectObject,
};
use windows_sys::Win32::Storage::Xps::PrintWindow;

use crate::error::{NativeError, api_error};

const PW_RENDER_FULL_CONTENT: u32 = 2;
const MAX_DIMENSION: u32 = 4_096;
const MAX_PIXELS: u64 = 8_294_400;
const MAX_PNG_BYTES: usize = 8_500_000;

pub fn capture_window_png(window_id: u64, width: u32, height: u32) -> Result<Vec<u8>, NativeError> {
    if width == 0
        || height == 0
        || width > MAX_DIMENSION
        || height > MAX_DIMENSION
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        return Err(NativeError::CaptureTooLarge);
    }
    let hwnd = usize::try_from(window_id).unwrap_or_default() as HWND;
    let width_i32 = i32::try_from(width).map_err(|_| NativeError::CaptureTooLarge)?;
    let height_i32 = i32::try_from(height).map_err(|_| NativeError::CaptureTooLarge)?;
    // SAFETY: the HWND has been identity-checked by the caller; returned handles are adopted below.
    let window_dc = unsafe { GetDC(hwnd) };
    if window_dc.is_null() {
        return Err(api_error("GetDC"));
    }
    // SAFETY: `window_dc` is valid and remains live in the cleanup guard.
    let memory_dc = unsafe { CreateCompatibleDC(window_dc) };
    // SAFETY: dimensions were bounded and `window_dc` remains valid.
    let bitmap = unsafe { CreateCompatibleBitmap(window_dc, width_i32, height_i32) };
    let mut guard = CaptureGuard::new(hwnd, window_dc, memory_dc, bitmap);
    if memory_dc.is_null() || bitmap.is_null() {
        return Err(api_error("CreateCompatibleDC/CreateCompatibleBitmap"));
    }
    // SAFETY: both GDI handles are valid and owned for the duration of capture.
    guard.previous = unsafe { SelectObject(memory_dc, bitmap) };
    // SAFETY: the validated HWND and compatible memory DC remain live.
    if unsafe { PrintWindow(hwnd, memory_dc, PW_RENDER_FULL_CONTENT) } == 0 {
        return Err(api_error("PrintWindow"));
    }
    let pixels = pixels(memory_dc, bitmap, width, height)?;
    encode_png(width, height, pixels)
}

fn pixels(
    memory_dc: HDC,
    bitmap: HBITMAP,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, NativeError> {
    let byte_count = usize::try_from(u64::from(width) * u64::from(height) * 4)
        .map_err(|_| NativeError::CaptureTooLarge)?;
    let mut pixels = vec![0_u8; byte_count];
    let mut info = BITMAPINFO::default();
    info.bmiHeader.biSize =
        u32::try_from(size_of::<windows_sys::Win32::Graphics::Gdi::BITMAPINFOHEADER>())
            .expect("bitmap header size");
    info.bmiHeader.biWidth = i32::try_from(width).expect("bounded width");
    info.bmiHeader.biHeight = -i32::try_from(height).expect("bounded height");
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB;
    info.bmiHeader.biSizeImage = u32::try_from(byte_count).unwrap_or(u32::MAX);
    // SAFETY: all GDI handles are live; pixel and BITMAPINFO buffers are correctly sized.
    let rows = unsafe {
        GetDIBits(
            memory_dc,
            bitmap,
            0,
            height,
            pixels.as_mut_ptr().cast(),
            &raw mut info,
            DIB_RGB_COLORS,
        )
    };
    if rows == i32::try_from(height).expect("bounded height") {
        Ok(pixels)
    } else {
        Err(NativeError::IncompleteCapture)
    }
}

fn encode_png(width: u32, height: u32, mut bgra: Vec<u8>) -> Result<Vec<u8>, NativeError> {
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let mut output = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut output, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&bgra)?;
    }
    if output.len() > MAX_PNG_BYTES {
        Err(NativeError::CaptureTooLarge)
    } else {
        Ok(output)
    }
}

struct CaptureGuard {
    window: isize,
    window_dc: isize,
    memory_dc: isize,
    bitmap: isize,
    previous: HGDIOBJ,
}

impl CaptureGuard {
    fn new(window: HWND, window_dc: HDC, memory_dc: HDC, bitmap: HBITMAP) -> Self {
        Self {
            window: window as isize,
            window_dc: window_dc as isize,
            memory_dc: memory_dc as isize,
            bitmap: bitmap as isize,
            previous: std::ptr::null_mut(),
        }
    }
}

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        // SAFETY: every non-null GDI handle is owned by this guard and released exactly once.
        unsafe {
            if !self.previous.is_null() && self.memory_dc != 0 {
                let _ = SelectObject(self.memory_dc as HDC, self.previous);
            }
            if self.bitmap != 0 {
                let _ = DeleteObject(self.bitmap as HGDIOBJ);
            }
            if self.memory_dc != 0 {
                let _ = DeleteDC(self.memory_dc as HDC);
            }
            if self.window_dc != 0 {
                let _ = ReleaseDC(self.window as HWND, self.window_dc as HDC);
            }
        }
    }
}
