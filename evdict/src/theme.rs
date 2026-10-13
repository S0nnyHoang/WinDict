//! Màu sáng/tối theo hệ thống (bảng màu lấy theo Windows 11 Fluent) và font.

use std::ptr::null_mut;
use std::sync::OnceLock;

use windows_sys::core::w;
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontIndirectW, DeleteObject, GetDC, GetTextFaceW, ReleaseDC, SelectObject, LOGFONTW,
};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

/// Màu dạng 0xRRGGBB.
#[derive(Clone, Copy)]
pub struct Theme {
    pub dark: bool,
    pub bg: u32,
    pub window_border: u32,
    pub field: u32,
    pub field_border: u32,
    pub field_bottom: u32,
    pub accent: u32,
    pub text: u32,
    pub text2: u32,
    pub example: u32,
    pub green: u32,
}

pub const LIGHT: Theme = Theme {
    dark: false,
    bg: 0xF9F9F9,
    window_border: 0xD5D5D5,
    field: 0xFFFFFF,
    field_border: 0xE3E3E3,
    field_bottom: 0x8A8A8A,
    accent: 0x005FB8,
    text: 0x1B1B1B,
    text2: 0x5F5F5F,
    example: 0x3A3A3A,
    green: 0x0F7B0F,
};

pub const DARK: Theme = Theme {
    dark: true,
    bg: 0x272727,
    window_border: 0x444444,
    field: 0x1E1E1E,
    field_border: 0x383838,
    field_bottom: 0x9A9A9A,
    accent: 0x60CDFF,
    text: 0xFFFFFF,
    text2: 0xA8A8A8,
    example: 0xD9D9D9,
    green: 0x6CCB5F,
};

/// Đổi 0xRRGGBB sang COLORREF (0x00BBGGRR) của GDI.
pub fn cr(rgb: u32) -> u32 {
    ((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF)
}

/// Windows đang dùng chế độ tối cho ứng dụng?
pub fn system_dark() -> bool {
    let mut v = 1u32;
    let mut size = 4u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            null_mut(),
            &mut v as *mut u32 as _,
            &mut size,
        )
    };
    r == 0 && v == 0
}

pub fn current() -> Theme {
    // EVDICT_THEME=dark|light ép theme (để kiểm thử), mặc định theo hệ thống.
    let forced = std::env::var("EVDICT_THEME").ok();
    let dark = match forced.as_deref() {
        Some("dark") => true,
        Some("light") => false,
        _ => system_dark(),
    };
    if dark {
        DARK
    } else {
        LIGHT
    }
}

/// Cho phép menu khay hệ thống và thanh cuộn theo chế độ tối của hệ thống.
/// Dùng API chưa công bố của uxtheme (theo số thứ tự), có sẵn từ Windows 10 1809.
pub fn allow_dark_mode() {
    unsafe {
        let ux = LoadLibraryW(w!("uxtheme.dll"));
        if ux.is_null() {
            return;
        }
        // 135: SetPreferredAppMode(AllowDark = 1); 136: FlushMenuThemes()
        if let Some(f) = GetProcAddress(ux, 135 as _) {
            let f: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(f);
            f(1);
        }
        if let Some(f) = GetProcAddress(ux, 136 as _) {
            let f: unsafe extern "system" fn() = std::mem::transmute(f);
            f();
        }
    }
}

/// Bật thanh cuộn tối cho một control (uxtheme 133: AllowDarkModeForWindow).
pub unsafe fn allow_dark_for_window(hwnd: windows_sys::Win32::Foundation::HWND, dark: bool) {
    let ux = LoadLibraryW(w!("uxtheme.dll"));
    if let Some(f) = GetProcAddress(ux, 133 as _) {
        let f: unsafe extern "system" fn(windows_sys::Win32::Foundation::HWND, i32) -> i32 =
            std::mem::transmute(f);
        f(hwnd, dark as i32);
    }
}

fn font_exists(name: &str) -> bool {
    unsafe {
        let mut lf: LOGFONTW = std::mem::zeroed();
        lf.lfHeight = -12;
        lf.lfCharSet = 1;
        for (i, c) in name.encode_utf16().take(31).enumerate() {
            lf.lfFaceName[i] = c;
        }
        let font = CreateFontIndirectW(&lf);
        let dc = GetDC(null_mut());
        let old = SelectObject(dc, font);
        let mut buf = [0u16; 64];
        let n = GetTextFaceW(dc, 64, buf.as_mut_ptr());
        SelectObject(dc, old);
        ReleaseDC(null_mut(), dc);
        DeleteObject(font);
        n > 0 && String::from_utf16_lossy(&buf[..n as usize - 1]).eq_ignore_ascii_case(name)
    }
}

pub struct Fonts {
    pub text: &'static str,
    pub display: &'static str,
    pub icons: &'static str,
}

/// Font của Windows 11 nếu có, không thì dùng font của Windows 10.
pub fn fonts() -> &'static Fonts {
    static F: OnceLock<Fonts> = OnceLock::new();
    F.get_or_init(|| {
        let variable = font_exists("Segoe UI Variable Text");
        Fonts {
            text: if variable { "Segoe UI Variable Text" } else { "Segoe UI" },
            display: if variable { "Segoe UI Variable Display" } else { "Segoe UI" },
            icons: if font_exists("Segoe Fluent Icons") { "Segoe Fluent Icons" } else { "Segoe MDL2 Assets" },
        }
    })
}
