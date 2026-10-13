//! Vẽ phần giao diện tự vẽ: hình bo góc khử răng cưa (không cần GDI+) và
//! ô nhập kiểu Fluent của Windows 11. Vẽ vào bộ đệm rồi chép một lần để không nhấp nháy.

use std::mem::{size_of, zeroed};
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;

use crate::theme::Theme;
use crate::util::sc;

/// Độ phủ (0..1) của hình chữ nhật bo góc tại tâm điểm ảnh.
fn coverage(px: f32, py: f32, x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let qx = (px - cx).abs() - (w / 2.0 - r);
    let qy = (py - cy).abs() - (h / 2.0 - r);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    let d = outside + qx.max(qy).min(0.0) - r;
    (0.5 - d).clamp(0.0, 1.0)
}

/// Tô hình chữ nhật bo góc khử răng cưa lên bộ đệm điểm ảnh 0xRRGGBB.
pub fn fill_round_rect(buf: &mut [u32], bw: i32, bh: i32, rect: (i32, i32, i32, i32), r: f32, color: u32) {
    let (x, y, w, h) = rect;
    let blend = |dst: u32, a: f32| -> u32 {
        let mix = |s: u32| {
            let (c, d) = (((color >> s) & 0xFF) as f32, ((dst >> s) & 0xFF) as f32);
            ((c * a + d * (1.0 - a)).round() as u32) << s
        };
        mix(16) | mix(8) | mix(0)
    };
    for py in y.max(0)..(y + h).min(bh) {
        for px in x.max(0)..(x + w).min(bw) {
            let a = coverage(px as f32 + 0.5, py as f32 + 0.5, x as f32, y as f32, w as f32, h as f32, r);
            if a > 0.0 {
                let i = (py * bw + px) as usize;
                buf[i] = if a >= 1.0 { color } else { blend(buf[i], a) };
            }
        }
    }
}

/// Ô nhập kiểu Fluent: viền mảnh, gạch dưới đậm hơn, màu nhấn khi có focus.
pub fn field(buf: &mut [u32], bw: i32, bh: i32, rect: (i32, i32, i32, i32), dpi: u32, t: &Theme, focused: bool) {
    let (x, y, w, h) = rect;
    let radius = sc(6, dpi) as f32;
    let bottom = if focused { sc(2, dpi) } else { 1 };
    fill_round_rect(buf, bw, bh, rect, radius, if focused { t.accent } else { t.field_bottom });
    fill_round_rect(buf, bw, bh, (x, y, w, h - bottom), radius, t.field_border);
    fill_round_rect(buf, bw, bh, (x + 1, y + 1, w - 2, h - bottom - 1), radius - 1.0, t.field);
}

/// Xử lý WM_PAINT: `shapes` vẽ lên bộ đệm điểm ảnh (đã tô nền `bg`),
/// sau đó `text` vẽ chữ bằng GDI lên cùng bộ đệm qua DC.
pub unsafe fn buffered_paint(
    hwnd: HWND,
    bg: u32,
    shapes: impl FnOnce(&mut [u32], i32, i32),
    text: impl FnOnce(HDC),
) {
    let mut ps: PAINTSTRUCT = zeroed();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut rc: RECT = zeroed();
    GetClientRect(hwnd, &mut rc);
    let (w, h) = (rc.right, rc.bottom);

    let mut bi: BITMAPINFO = zeroed();
    bi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
    bi.bmiHeader.biWidth = w;
    bi.bmiHeader.biHeight = -h;
    bi.bmiHeader.biPlanes = 1;
    bi.bmiHeader.biBitCount = 32;
    let mut bits: *mut core::ffi::c_void = null_mut();
    let dib = CreateDIBSection(hdc, &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    if !dib.is_null() && w > 0 && h > 0 {
        let buf = std::slice::from_raw_parts_mut(bits as *mut u32, (w * h) as usize);
        buf.fill(bg);
        shapes(buf, w, h);
        let mem = CreateCompatibleDC(hdc);
        let old = SelectObject(mem, dib);
        SetBkMode(mem, TRANSPARENT as _);
        text(mem);
        BitBlt(hdc, 0, 0, w, h, mem, 0, 0, SRCCOPY);
        SelectObject(mem, old);
        DeleteDC(mem);
    }
    if !dib.is_null() {
        DeleteObject(dib);
    }
    EndPaint(hwnd, &ps);
}
