//! Cấu hình lưu trong HKCU\Software\EVDict, khởi động cùng Windows qua khoá Run,
//! và hộp thoại đổi phím tắt.

use std::cell::Cell;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use windows_sys::core::w;
use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows_sys::Win32::Graphics::Gdi::{
    CreateSolidBrush, DeleteObject, DrawTextW, FillRect, InvalidateRect, SelectObject, SetBkColor,
    SetTextColor, DT_LEFT, DT_SINGLELINE, DT_VCENTER, HBRUSH, HDC,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_DWORD, REG_SZ,
    RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};
use windows_sys::Win32::UI::Controls::SetWindowTheme;
use windows_sys::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForSystem};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, GetKeyNameTextW, GetKeyState, MapVirtualKeyW, RegisterHotKey, SetFocus,
    UnregisterHotKey,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::draw;
use crate::theme::{self, cr, Theme};
use crate::util::*;
use crate::{app, update_tray, HOTKEY_ID};

pub const MOD_ALT: u32 = 1;
pub const MOD_CONTROL: u32 = 2;
pub const MOD_SHIFT: u32 = 4;
pub const MOD_WIN: u32 = 8;
const MOD_NOREPEAT: u32 = 0x4000;
pub const DEFAULT_HOTKEY: (u32, u32) = (MOD_CONTROL | MOD_ALT, 0x44); // Ctrl+Alt+D


const ID_SAVE: usize = 1;
const ID_CANCEL: usize = 2;
const ID_DEFAULT: usize = 3;

// ---------- Registry ----------

unsafe fn reg_dword(name: &str) -> Option<u32> {
    let name = wide(name);
    let mut v = 0u32;
    let mut size = 4u32;
    let r = RegGetValueW(
        HKEY_CURRENT_USER,
        w!("Software\\EVDict"),
        name.as_ptr(),
        RRF_RT_REG_DWORD,
        null_mut(),
        &mut v as *mut u32 as _,
        &mut size,
    );
    (r == 0).then_some(v)
}

unsafe fn set_reg_dword(name: &str, v: u32) {
    let name = wide(name);
    RegSetKeyValueW(
        HKEY_CURRENT_USER,
        w!("Software\\EVDict"),
        name.as_ptr(),
        REG_DWORD,
        &v as *const u32 as _,
        4,
    );
}

pub fn load_hotkey() -> (u32, u32) {
    unsafe {
        match (reg_dword("HotkeyMods"), reg_dword("HotkeyVk")) {
            (Some(m), Some(vk)) if vk != 0 && vk < 256 => (m & 0xF, vk),
            _ => DEFAULT_HOTKEY,
        }
    }
}

fn save_hotkey(mods: u32, vk: u32) {
    unsafe {
        set_reg_dword("HotkeyMods", mods);
        set_reg_dword("HotkeyVk", vk);
    }
}

const RUN_KEY: *const u16 = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");

pub fn autostart_enabled() -> bool {
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            w!("EVDict"),
            RRF_RT_REG_SZ,
            null_mut(),
            null_mut(),
            null_mut(),
        ) == 0
    }
}

pub fn set_autostart(on: bool) {
    unsafe {
        if on {
            let v = wide(&format!("\"{}\"", exe_path()));
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                w!("EVDict"),
                REG_SZ,
                v.as_ptr() as _,
                (v.len() * 2) as u32,
            );
        } else {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, w!("EVDict"));
        }
    }
}

// ---------- Phím tắt ----------

pub unsafe fn register_hotkey(main: HWND, mods: u32, vk: u32) -> bool {
    let ok = RegisterHotKey(main, HOTKEY_ID, mods | MOD_NOREPEAT, vk) != 0;
    app(|a| a.hk_registered.set(ok));
    ok
}

pub unsafe fn unregister_hotkey(main: HWND) {
    UnregisterHotKey(main, HOTKEY_ID);
    app(|a| a.hk_registered.set(false));
}

pub fn hotkey_name(mods: u32, vk: u32) -> String {
    let mut parts: Vec<String> = Vec::new();
    for (m, n) in [(MOD_CONTROL, "Ctrl"), (MOD_ALT, "Alt"), (MOD_SHIFT, "Shift"), (MOD_WIN, "Win")] {
        if mods & m != 0 {
            parts.push(n.into());
        }
    }
    if vk == 0 {
        return parts.join("+");
    }
    let key = match vk {
        0x30..=0x39 | 0x41..=0x5A => char::from(vk as u8).to_string(),
        _ => unsafe {
            let mut sc = MapVirtualKeyW(vk, 0) as i32;
            // Phím mở rộng (mũi tên, Home, End, Insert, Delete...) cần cờ bit 24.
            if matches!(vk, 0x21..=0x28 | 0x2D | 0x2E | 0x6F | 0x90) {
                sc |= 0x100;
            }
            let mut buf = [0u16; 64];
            let n = GetKeyNameTextW(sc << 16, buf.as_mut_ptr(), 64);
            if n > 0 {
                String::from_utf16_lossy(&buf[..n as usize])
            } else {
                format!("0x{vk:02X}")
            }
        },
    };
    parts.push(key);
    parts.join("+")
}

pub fn current_hotkey_name() -> String {
    let (m, vk) = app(|a| (a.hk_mods.get(), a.hk_vk.get()));
    hotkey_name(m, vk)
}

// ---------- Hộp thoại đổi phím tắt ----------

thread_local! {
    static DLG_THEME: Cell<Theme> = const { Cell::new(theme::LIGHT) };
    static DLG_BRUSH: Cell<HBRUSH> = const { Cell::new(null_mut()) };
    /// Tổ hợp đang chọn trong ô nhập: (MOD_*, mã phím).
    static KEY: Cell<(u32, u32)> = const { Cell::new((0, 0)) };
    /// Các phím bổ trợ đang được giữ khi người dùng đang bấm (để hiện "Ctrl + Alt + …").
    static HELD: Cell<u32> = const { Cell::new(0) };
}

pub unsafe fn register_class(hinst: HINSTANCE) {
    let mut wc: WNDCLASSEXW = zeroed();
    wc.cbSize = size_of::<WNDCLASSEXW>() as u32;
    wc.lpfnWndProc = Some(wndproc);
    wc.hInstance = hinst;
    wc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
    wc.lpszClassName = w!("EVDictSettings");
    RegisterClassExW(&wc);

    wc.lpfnWndProc = Some(keyfield_proc);
    wc.hCursor = LoadCursorW(null_mut(), IDC_IBEAM);
    wc.lpszClassName = w!("EVDictKeyField");
    RegisterClassExW(&wc);
}

pub unsafe fn open() {
    let existing = app(|a| a.settings.get());
    if !existing.is_null() {
        force_foreground(existing, app(|a| a.hkctrl.get()));
        return;
    }
    let main = app(|a| a.main.get());
    // Tạm gỡ phím tắt để ô nhập nhận được tổ hợp phím, kể cả tổ hợp đang dùng.
    unregister_hotkey(main);

    let dpi = GetDpiForSystem();
    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU;
    let ex = WS_EX_TOPMOST | WS_EX_DLGMODALFRAME;
    let mut rc = RECT { left: 0, top: 0, right: sc(360, dpi), bottom: sc(148, dpi) };
    AdjustWindowRectExForDpi(&mut rc, style, 0, ex, dpi);
    let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
    let x = (GetSystemMetrics(SM_CXSCREEN) - w) / 2;
    let y = (GetSystemMetrics(SM_CYSCREEN) - h) / 2;

    let hinst = GetModuleHandleW(null());
    let hwnd = CreateWindowExW(
        ex, w!("EVDictSettings"), w!("Đổi phím tắt tra từ"), style,
        x, y, w, h, null_mut(), null_mut(), hinst, null(),
    );
    let t = theme::current();
    let dark: i32 = t.dark as i32;
    DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE as u32, &dark as *const i32 as _, 4);
    DLG_THEME.with(|c| c.set(t));
    DLG_BRUSH.with(|b| b.set(CreateSolidBrush(cr(t.bg))));
    KEY.with(|k| k.set(app(|a| (a.hk_mods.get(), a.hk_vk.get()))));
    HELD.with(|k| k.set(0));

    let font = make_font(theme::fonts().text, 10, dpi);
    app(|a| a.settings_font.set(font));
    let child = |class: *const u16, text: *const u16, style: u32, x: i32, y: i32, cw: i32, ch: i32, id: usize| {
        let c = CreateWindowExW(
            0, class, text, WS_CHILD | WS_VISIBLE | style,
            sc(x, dpi), sc(y, dpi), sc(cw, dpi), sc(ch, dpi),
            hwnd, id as _, hinst, null(),
        );
        set_font(c, font);
        theme::allow_dark_for_window(c, t.dark);
        SetWindowTheme(c, if t.dark { w!("DarkMode_Explorer") } else { null() }, null());
        c
    };
    child(w!("STATIC"), w!("Bấm tổ hợp phím mới (ví dụ Ctrl+Alt+D):"), 0, 16, 14, 328, 20, 0);
    let field = child(w!("EVDictKeyField"), null(), WS_TABSTOP, 16, 40, 328, 34, 10);
    child(w!("BUTTON"), w!("Mặc định"), WS_TABSTOP, 16, 98, 100, 32, ID_DEFAULT);
    child(w!("BUTTON"), w!("Lưu"), WS_TABSTOP | 1 /* BS_DEFPUSHBUTTON */, 148, 98, 96, 32, ID_SAVE);
    child(w!("BUTTON"), w!("Huỷ"), WS_TABSTOP, 252, 98, 92, 32, ID_CANCEL);

    app(|a| {
        a.settings.set(hwnd);
        a.hkctrl.set(field);
    });
    force_foreground(hwnd, field);
}

unsafe fn save(hwnd: HWND) {
    let main = app(|a| a.main.get());
    let (mods, vk) = KEY.with(|k| k.get());
    if vk == 0 {
        MessageBoxW(hwnd, w!("Bạn chưa bấm tổ hợp phím nào."), w!("Từ điển"), MB_ICONWARNING);
        return;
    }
    if !register_hotkey(main, mods, vk) {
        let text = wide(&format!(
            "Tổ hợp {} đang được Windows hoặc ứng dụng khác sử dụng. Hãy chọn tổ hợp khác.",
            hotkey_name(mods, vk)
        ));
        MessageBoxW(hwnd, text.as_ptr(), w!("Từ điển"), MB_ICONWARNING);
        return;
    }
    app(|a| {
        a.hk_mods.set(mods);
        a.hk_vk.set(vk);
    });
    save_hotkey(mods, vk);
    update_tray(false);
    DestroyWindow(hwnd);
}

pub unsafe fn handle_key(msg: &MSG) -> bool {
    let s = app(|a| a.settings.get());
    if s.is_null() || (msg.hwnd != s && IsChild(s, msg.hwnd) == 0) {
        return false;
    }
    match msg.wParam {
        0x1B => {
            DestroyWindow(s);
            true
        }
        0x0D if held_mods() == 0 => {
            save(s);
            true
        }
        _ => false,
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => {
            let mut rc: RECT = zeroed();
            GetClientRect(hwnd, &mut rc);
            FillRect(wp as HDC, &rc, DLG_BRUSH.with(|b| b.get()));
            1
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
            let t = DLG_THEME.with(|c| c.get());
            SetTextColor(wp as HDC, cr(t.text));
            SetBkColor(wp as HDC, cr(t.bg));
            DLG_BRUSH.with(|b| b.get()) as LRESULT
        }
        WM_COMMAND => {
            match wp & 0xFFFF {
                ID_SAVE => save(hwnd),
                ID_CANCEL => {
                    DestroyWindow(hwnd);
                }
                ID_DEFAULT => {
                    KEY.with(|k| k.set(DEFAULT_HOTKEY));
                    let field = app(|a| a.hkctrl.get());
                    InvalidateRect(field, null(), 0);
                    SetFocus(field);
                }
                _ => {}
            }
            0
        }
        WM_DESTROY => {
            let (main, registered, m, vk, font) = app(|a| {
                a.settings.set(null_mut());
                a.hkctrl.set(null_mut());
                (a.main.get(), a.hk_registered.get(), a.hk_mods.get(), a.hk_vk.get(), a.settings_font.replace(null_mut()))
            });
            DeleteObject(font);
            DeleteObject(DLG_BRUSH.with(|b| b.replace(null_mut())));
            if !registered {
                // Huỷ: đăng ký lại phím tắt cũ.
                register_hotkey(main, m, vk);
            }
            trim_memory();
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

// ---------- Ô nhập tổ hợp phím (tự vẽ, theo theme) ----------

unsafe fn held_mods() -> u32 {
    let down = |vk: u16| GetKeyState(vk as i32) as u16 & 0x8000 != 0;
    let mut m = 0;
    if down(0x11) { m |= MOD_CONTROL; }
    if down(0x12) { m |= MOD_ALT; }
    if down(0x10) { m |= MOD_SHIFT; }
    if down(0x5B) || down(0x5C) { m |= MOD_WIN; }
    m
}

fn is_modifier(vk: u32) -> bool {
    matches!(vk, 0x10..=0x12 | 0x5B | 0x5C | 0xA0..=0xA5)
}

unsafe extern "system" fn keyfield_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let vk = wp as u32;
            let mods = held_mods();
            if is_modifier(vk) {
                HELD.with(|h| h.set(mods));
            } else if mods & (MOD_CONTROL | MOD_ALT | MOD_WIN) != 0 {
                // Cần ít nhất Ctrl, Alt hoặc Win để không chặn phím gõ bình thường.
                KEY.with(|k| k.set((mods, vk)));
                HELD.with(|h| h.set(0));
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_KEYUP | WM_SYSKEYUP => {
            HELD.with(|h| h.set(held_mods()));
            InvalidateRect(hwnd, null(), 0);
            0
        }
        // Không để Alt bật menu hay tạo ký tự.
        WM_CHAR | WM_SYSCHAR | WM_DEADCHAR | WM_SYSDEADCHAR => 0,
        WM_SETFOCUS | WM_KILLFOCUS => {
            HELD.with(|h| h.set(0));
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_LBUTTONDOWN => {
            SetFocus(hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let t = DLG_THEME.with(|c| c.get());
            let dpi = GetDpiForSystem();
            let focused = GetFocus() == hwnd;
            let held = HELD.with(|h| h.get());
            let (mods, vk) = KEY.with(|k| k.get());
            let (text, color) = if held != 0 {
                (format!("{}+…", hotkey_name(held, 0)), t.text2)
            } else if vk != 0 {
                (hotkey_name(mods, vk), t.text)
            } else {
                ("Bấm tổ hợp phím…".to_string(), t.text2)
            };
            let text = text.replace('+', " + ");
            let font = app(|a| a.settings_font.get());
            draw::buffered_paint(
                hwnd,
                t.bg,
                |buf, w, h| draw::field(buf, w, h, (0, 0, w, h), dpi, &t, focused),
                |dc| {
                    let mut rc: RECT = zeroed();
                    GetClientRect(hwnd, &mut rc);
                    rc.left += sc(12, dpi);
                    rc.bottom -= if focused { sc(2, dpi) } else { 1 };
                    SelectObject(dc, font);
                    SetTextColor(dc, cr(color));
                    let mut s: Vec<u16> = text.encode_utf16().collect();
                    DrawTextW(dc, s.as_mut_ptr(), s.len() as i32, &mut rc, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
                },
            );
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}