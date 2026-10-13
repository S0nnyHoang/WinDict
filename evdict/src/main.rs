#![windows_subsystem = "windows"]

//! Từ điển Anh–Việt chạy nền: bôi đen một từ ở bất kỳ đâu, bấm phím tắt
//! (mặc định Ctrl+Alt+D) để xem nghĩa. Khi nghỉ chỉ chờ GetMessage, không tốn CPU.

mod clip;
mod dict;
mod draw;
mod key;
mod library;
mod manager;
mod popup;
mod render;
mod settings;
mod stardict;
mod theme;
mod util;

use std::cell::Cell;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use windows_sys::core::w;
use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::DataExchange::COPYDATASTRUCT;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::HiDpi::{
    GetDpiForSystem, GetSystemMetricsForDpi, SetProcessDpiAwarenessContext,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use util::{wide, EM_SETSEL};

pub const HOTKEY_ID: i32 = 1;
const WM_TRAY: u32 = WM_APP + 1;
const WM_APP_SHOW: u32 = WM_APP + 2;
const WM_APP_SETTINGS: u32 = WM_APP + 3;
const WM_APP_DICTS: u32 = WM_APP + 4;

pub struct App {
    pub main: Cell<HWND>,
    pub popup: Cell<HWND>,
    pub edit: Cell<HWND>,
    pub rich: Cell<HWND>,
    pub popup_font: Cell<HFONT>,
    pub popup_dpi: Cell<u32>,
    pub settings: Cell<HWND>,
    pub hkctrl: Cell<HWND>,
    pub settings_font: Cell<HFONT>,
    pub hk_mods: Cell<u32>,
    pub hk_vk: Cell<u32>,
    pub hk_registered: Cell<bool>,
    pub icon: Cell<HICON>,
    pub taskbar_msg: Cell<u32>,
}

thread_local! {
    static APP: App = App {
        main: Cell::new(null_mut()),
        popup: Cell::new(null_mut()),
        edit: Cell::new(null_mut()),
        rich: Cell::new(null_mut()),
        popup_font: Cell::new(null_mut()),
        popup_dpi: Cell::new(96),
        settings: Cell::new(null_mut()),
        hkctrl: Cell::new(null_mut()),
        settings_font: Cell::new(null_mut()),
        hk_mods: Cell::new(0),
        hk_vk: Cell::new(0),
        hk_registered: Cell::new(false),
        icon: Cell::new(null_mut()),
        taskbar_msg: Cell::new(0),
    };
}

pub fn app<R>(f: impl FnOnce(&App) -> R) -> R {
    APP.with(f)
}

fn main() {
    unsafe { run() }
}

unsafe fn run() {
    // Chỉ chạy một bản; mở lần nữa thì bật ô tra từ của bản đang chạy.
    // `evdict.exe <từ>` mở popup tra ngay từ đó; `evdict.exe --settings` mở hộp thoại phím tắt.
    let arg: Option<String> = {
        let rest: Vec<String> = std::env::args().skip(1).collect();
        (!rest.is_empty()).then(|| rest.join(" "))
    };
    CreateMutexW(null(), 0, w!("Local\\EVDict.SingleInstance"));
    if GetLastError() == ERROR_ALREADY_EXISTS {
        let other = FindWindowW(w!("EVDictMain"), null());
        if !other.is_null() {
            match arg {
                Some(a) if a == "--settings" || a == "--dicts" => {
                    AllowSetForegroundWindow(ASFW_ANY);
                    let m = if a == "--dicts" { WM_APP_DICTS } else { WM_APP_SETTINGS };
                    PostMessageW(other, m, 0, 0);
                }
                Some(a) if a.starts_with("--import ") => {
                    let data: Vec<u16> = a["--import ".len()..].encode_utf16().collect();
                    let cds = COPYDATASTRUCT { dwData: 2, cbData: (data.len() * 2) as u32, lpData: data.as_ptr() as _ };
                    AllowSetForegroundWindow(ASFW_ANY);
                    SendMessageW(other, WM_COPYDATA, 0, &cds as *const _ as LPARAM);
                }
                Some(word) => {
                    let data: Vec<u16> = word.encode_utf16().collect();
                    let cds = COPYDATASTRUCT {
                        dwData: 1,
                        cbData: (data.len() * 2) as u32,
                        lpData: data.as_ptr() as _,
                    };
                    AllowSetForegroundWindow(ASFW_ANY);
                    SendMessageW(other, WM_COPYDATA, 0, &cds as *const _ as LPARAM);
                }
                None => {
                    AllowSetForegroundWindow(ASFW_ANY);
                    PostMessageW(other, WM_APP_SHOW, 0, 0);
                }
            }
        }
        return;
    }

    SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    theme::allow_dark_mode();
    let hinst = GetModuleHandleW(null());

    let mut wc: WNDCLASSEXW = zeroed();
    wc.cbSize = size_of::<WNDCLASSEXW>() as u32;
    wc.lpfnWndProc = Some(main_wndproc);
    wc.hInstance = hinst;
    wc.lpszClassName = w!("EVDictMain");
    RegisterClassExW(&wc);
    popup::register_class(hinst);
    settings::register_class(hinst);
    manager::register_class(hinst);

    let main = CreateWindowExW(
        0, w!("EVDictMain"), w!("EVDict"), WS_OVERLAPPED,
        0, 0, 0, 0, null_mut(), null_mut(), hinst, null(),
    );
    let (mods, vk) = settings::load_hotkey();
    app(|a| {
        a.main.set(main);
        a.hk_mods.set(mods);
        a.hk_vk.set(vk);
        a.taskbar_msg.set(RegisterWindowMessageW(w!("TaskbarCreated")));
        a.icon.set(make_icon());
    });
    update_tray(true);

    if !settings::register_hotkey(main, mods, vk) {
        let text = wide(&format!(
            "Phím tắt {} đang được ứng dụng khác sử dụng.\nHãy chọn phím tắt khác.",
            settings::hotkey_name(mods, vk)
        ));
        MessageBoxW(null_mut(), text.as_ptr(), w!("Từ điển Anh-Việt"), MB_ICONWARNING);
        settings::open();
    }

    match arg {
        Some(a) if a == "--settings" => settings::open(),
        Some(a) if a == "--dicts" => manager::open(),
        Some(a) if a.starts_with("--import ") => manager::open_and_import(vec![a["--import ".len()..].into()]),
        Some(word) => popup::show(Some(word), popup::HINT_TYPE),
        None => util::trim_memory(),
    }

    let mut msg: MSG = zeroed();
    while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
        dispatch(&msg);
    }
}

/// Xử lý một thông điệp; dùng chung cho vòng lặp chính và vòng lặp hoạt ảnh cuộn.
pub unsafe fn dispatch(msg: &MSG) {
    let handled = match msg.message {
        WM_KEYDOWN => popup::handle_key(msg) || settings::handle_key(msg) || manager::handle_key(msg),
        WM_MOUSEWHEEL => popup::handle_wheel(msg),
        _ => false,
    };
    if !handled {
        TranslateMessage(msg);
        DispatchMessageW(msg);
    }
}

pub unsafe fn update_tray(add: bool) {
    let (main, icon) = app(|a| (a.main.get(), a.icon.get()));
    let mut nid: NOTIFYICONDATAW = zeroed();
    nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = main;
    nid.uID = 1;
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY;
    nid.hIcon = icon;
    let tip = format!("Từ điển Anh-Việt ({})", settings::current_hotkey_name());
    for (i, c) in tip.encode_utf16().take(127).enumerate() {
        nid.szTip[i] = c;
    }
    Shell_NotifyIconW(if add { NIM_ADD } else { NIM_MODIFY }, &nid);
}

unsafe fn remove_tray() {
    let mut nid: NOTIFYICONDATAW = zeroed();
    nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = app(|a| a.main.get());
    nid.uID = 1;
    Shell_NotifyIconW(NIM_DELETE, &nid);
}

/// Vẽ icon khay hệ thống: ô vuông bo góc màu xanh với chữ "D".
unsafe fn make_icon() -> HICON {
    let size = GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem());
    let screen = GetDC(null_mut());
    let dc = CreateCompatibleDC(screen);
    let color = CreateCompatibleBitmap(screen, size, size);
    let mask = CreateBitmap(size, size, 1, 1, null());
    let rc = RECT { left: 0, top: 0, right: size, bottom: size };
    let radius = size / 3;

    let old_bmp = SelectObject(dc, color);
    FillRect(dc, &rc, GetStockObject(BLACK_BRUSH) as HBRUSH);
    let brush = CreateSolidBrush(0x00B16300); // BGR: xanh dương
    let old_brush = SelectObject(dc, brush);
    let old_pen = SelectObject(dc, GetStockObject(NULL_PEN));
    RoundRect(dc, 0, 0, size + 1, size + 1, radius, radius);

    let mut lf: LOGFONTW = zeroed();
    lf.lfHeight = -(size * 4 / 5);
    lf.lfWeight = 700;
    lf.lfQuality = 4; // ANTIALIASED_QUALITY
    for (i, c) in "Segoe UI".encode_utf16().enumerate() {
        lf.lfFaceName[i] = c;
    }
    let font = CreateFontIndirectW(&lf);
    let old_font = SelectObject(dc, font);
    SetBkMode(dc, TRANSPARENT as _);
    SetTextColor(dc, 0x00FFFFFF);
    let mut text = [b'D' as u16];
    let mut trc = rc;
    DrawTextW(dc, text.as_mut_ptr(), 1, &mut trc, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

    SelectObject(dc, mask);
    FillRect(dc, &rc, GetStockObject(WHITE_BRUSH) as HBRUSH);
    SelectObject(dc, GetStockObject(BLACK_BRUSH));
    RoundRect(dc, 0, 0, size + 1, size + 1, radius, radius);

    SelectObject(dc, old_font);
    SelectObject(dc, old_pen);
    SelectObject(dc, old_brush);
    SelectObject(dc, old_bmp);

    let info = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
    let icon = CreateIconIndirect(&info);

    DeleteObject(font);
    DeleteObject(brush);
    DeleteObject(color);
    DeleteObject(mask);
    DeleteDC(dc);
    ReleaseDC(null_mut(), screen);
    icon
}

unsafe fn show_menu(hwnd: HWND) {
    let menu = CreatePopupMenu();
    let lookup = wide(&format!("Tra từ\t{}", settings::current_hotkey_name()));
    AppendMenuW(menu, MF_STRING, 1, lookup.as_ptr());
    AppendMenuW(menu, MF_STRING, 5, w!("Quản lý từ điển..."));
    AppendMenuW(menu, MF_STRING, 2, w!("Đổi phím tắt..."));
    let checked = if settings::autostart_enabled() { MF_CHECKED } else { MF_UNCHECKED };
    AppendMenuW(menu, MF_STRING | checked, 3, w!("Khởi động cùng Windows"));
    AppendMenuW(menu, MF_SEPARATOR, 0, null());
    AppendMenuW(menu, MF_STRING, 4, w!("Thoát"));

    let mut pt: POINT = zeroed();
    GetCursorPos(&mut pt);
    SetForegroundWindow(hwnd);
    let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, null());
    PostMessageW(hwnd, WM_NULL, 0, 0);
    DestroyMenu(menu);

    match cmd {
        1 => popup::show(None, popup::HINT_TYPE),
        2 => settings::open(),
        5 => manager::open(),
        3 => settings::set_autostart(!settings::autostart_enabled()),
        4 => {
            DestroyWindow(hwnd);
        }
        _ => {}
    }
}


unsafe fn on_hotkey(main: HWND) {
    let (popup, edit) = app(|a| (a.popup.get(), a.edit.get()));
    if !popup.is_null() && GetForegroundWindow() == popup {
        windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(edit);
        SendMessageW(edit, EM_SETSEL, 0, -1);
        return;
    }
    util::trace_start();
    let mut cap = clip::capture_selection(main);
    popup::show(cap.text.take(), popup::HINT_NOSEL);
    util::trace("popup shown");
    cap.restore(main);
}

unsafe extern "system" fn main_wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_HOTKEY if wp as i32 == HOTKEY_ID => {
            on_hotkey(hwnd);
            0
        }
        WM_TRAY => {
            match lp as u32 {
                WM_LBUTTONUP => popup::show(None, popup::HINT_TYPE),
                WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd),
                _ => {}
            }
            0
        }
        WM_SETTINGCHANGE => {
            // Đổi theme sáng/tối: cập nhật menu khay. Popup tự theo theme ở lần mở sau.
            theme::allow_dark_mode();
            0
        }
        WM_APP_SHOW => {
            popup::show(None, popup::HINT_TYPE);
            0
        }
        WM_APP_SETTINGS => {
            settings::open();
            0
        }
        WM_APP_DICTS => {
            manager::open();
            0
        }
        manager::WM_APP_IMPORT_PROGRESS => {
            manager::on_progress(wp);
            0
        }
        manager::WM_APP_IMPORT_DONE => {
            manager::on_done(lp);
            0
        }
        WM_COPYDATA => {
            let cds = &*(lp as *const COPYDATASTRUCT);
            if cds.dwData == 1 && !cds.lpData.is_null() {
                let s = std::slice::from_raw_parts(cds.lpData as *const u16, cds.cbData as usize / 2);
                let word = String::from_utf16_lossy(s);
                // Trả lời ngay để tiến trình gửi không phải chờ popup.
                ReplyMessage(1);
                popup::show(Some(word), popup::HINT_TYPE);
            } else if cds.dwData == 2 && !cds.lpData.is_null() {
                let s = std::slice::from_raw_parts(cds.lpData as *const u16, cds.cbData as usize / 2);
                let path = String::from_utf16_lossy(s);
                ReplyMessage(1);
                manager::open_and_import(vec![path.into()]);
            }
            1
        }
        // Đăng xuất/tắt máy, hoặc trình cài đặt yêu cầu đóng (Restart Manager): thoát gọn.
        WM_ENDSESSION if wp != 0 => {
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            remove_tray();
            settings::unregister_hotkey(hwnd);
            PostQuitMessage(0);
            0
        }
        _ if msg != 0 && msg == app(|a| a.taskbar_msg.get()) => {
            // Explorer khởi động lại: thêm lại icon.
            update_tray(true);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
