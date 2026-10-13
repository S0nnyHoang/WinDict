//! Cửa sổ "Quản lý từ điển": bật/tắt, đổi thứ tự ưu tiên, đổi tên (F2), xoá,
//! và thêm từ điển StarDict. Import chạy ở luồng phụ; kết quả gửi về cửa sổ chính
//! (WM_APP_IMPORT_*) nên vẫn được ghi nhận kể cả khi cửa sổ này đã đóng.

use std::cell::Cell;
use std::mem::{size_of, zeroed};
use std::path::PathBuf;
use std::ptr::{null, null_mut};

use windows_sys::core::w;
use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows_sys::Win32::Graphics::Gdi::{
    CreateSolidBrush, DeleteObject, SetBkColor, SetTextColor, HBRUSH, HDC, HFONT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, OPENFILENAMEW};
use windows_sys::Win32::UI::Controls::{
    InitCommonControlsEx, SetWindowTheme, INITCOMMONCONTROLSEX, LVCOLUMNW, LVITEMW, NMHDR,
    NMLISTVIEW, NMLVDISPINFOW,
};
use windows_sys::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForSystem};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::draw;
use crate::library::{self, Imported, BUILTIN};
use crate::theme::{self, cr, Theme};
use crate::util::*;
use crate::app;

pub const WM_APP_IMPORT_PROGRESS: u32 = WM_APP + 20;
pub const WM_APP_IMPORT_DONE: u32 = WM_APP + 21;

const ID_LIST: usize = 10;
const ID_ADD: usize = 11;
const ID_REMOVE: usize = 12;
const ID_UP: usize = 13;
const ID_DOWN: usize = 14;
const ID_CLOSE: usize = 15;

// ListView
const LVM_SETBKCOLOR: u32 = 0x1001;
const LVM_DELETEALLITEMS: u32 = 0x1009;
const LVM_GETNEXTITEM: u32 = 0x100C;
const LVM_ENSUREVISIBLE: u32 = 0x1013;
const LVM_SETTEXTCOLOR: u32 = 0x1024;
const LVM_SETTEXTBKCOLOR: u32 = 0x1026;
const LVM_SETITEMSTATE: u32 = 0x102B;
const LVM_SETEXTENDEDLISTVIEWSTYLE: u32 = 0x1036;
const LVM_INSERTITEMW: u32 = 0x104D;
const LVM_INSERTCOLUMNW: u32 = 0x1061;
const LVM_SETITEMTEXTW: u32 = 0x1074;
const LVM_EDITLABELW: u32 = 0x1076;
const LVS_REPORT: u32 = 0x1;
const LVS_SINGLESEL: u32 = 0x4;
const LVS_SHOWSELALWAYS: u32 = 0x8;
const LVS_EDITLABELS: u32 = 0x200;
const LVS_NOCOLUMNHEADER: u32 = 0x4000;
const LVS_EX_CHECKBOXES: isize = 0x4;
const LVS_EX_FULLROWSELECT: isize = 0x20;
const LVS_EX_LABELTIP: isize = 0x4000;
const LVS_EX_DOUBLEBUFFER: isize = 0x10000;
const LVIF_TEXT: u32 = 0x1;
const LVIF_STATE: u32 = 0x8;
const LVIS_SELECTED: u32 = 0x2;
const LVIS_FOCUSED: u32 = 0x1;
const LVIS_STATEIMAGEMASK: u32 = 0xF000;
const LVNI_SELECTED: usize = 0x2;
const LVN_ITEMCHANGED: u32 = (-101i32) as u32;
const LVN_ENDLABELEDITW: u32 = (-176i32) as u32;

struct State {
    hwnd: Cell<HWND>,
    list: Cell<HWND>,
    status: Cell<HWND>,
    font: Cell<HFONT>,
    brush: Cell<HBRUSH>,
    theme: Cell<Theme>,
    /// Đang đổ dữ liệu vào danh sách: bỏ qua thông báo đổi trạng thái ô chọn.
    filling: Cell<bool>,
    busy: Cell<bool>,
}

thread_local! {
    static M: State = const {
        State {
            hwnd: Cell::new(null_mut()),
            list: Cell::new(null_mut()),
            status: Cell::new(null_mut()),
            font: Cell::new(null_mut()),
            brush: Cell::new(null_mut()),
            theme: Cell::new(theme::LIGHT),
            filling: Cell::new(false),
            busy: Cell::new(false),
        }
    };
}

fn m<R>(f: impl FnOnce(&State) -> R) -> R {
    M.with(f)
}

pub unsafe fn register_class(hinst: HINSTANCE) {
    let icc = INITCOMMONCONTROLSEX { dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32, dwICC: 0x1 /* ICC_LISTVIEW_CLASSES */ };
    InitCommonControlsEx(&icc);
    let mut wc: WNDCLASSEXW = zeroed();
    wc.cbSize = size_of::<WNDCLASSEXW>() as u32;
    wc.lpfnWndProc = Some(wndproc);
    wc.hInstance = hinst;
    wc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
    wc.lpszClassName = w!("EVDictManager");
    RegisterClassExW(&wc);
}

pub unsafe fn open() {
    let existing = m(|s| s.hwnd.get());
    if !existing.is_null() {
        force_foreground(existing, m(|s| s.list.get()));
        return;
    }
    let dpi = GetDpiForSystem();
    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN;
    let ex = WS_EX_DLGMODALFRAME;
    let mut rc = RECT { left: 0, top: 0, right: sc(500, dpi), bottom: sc(396, dpi) };
    AdjustWindowRectExForDpi(&mut rc, style, 0, ex, dpi);
    let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
    let x = (GetSystemMetrics(SM_CXSCREEN) - w) / 2;
    let y = (GetSystemMetrics(SM_CYSCREEN) - h) / 2;
    let hinst = GetModuleHandleW(null());
    let hwnd = CreateWindowExW(
        ex, w!("EVDictManager"), w!("Quản lý từ điển"), style,
        x, y, w, h, null_mut(), null_mut(), hinst, null(),
    );

    let t = theme::current();
    let dark: i32 = t.dark as i32;
    DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE as u32, &dark as *const i32 as _, 4);
    let font = make_font(theme::fonts().text, 10, dpi);
    m(|s| {
        s.hwnd.set(hwnd);
        s.font.set(font);
        s.brush.set(CreateSolidBrush(cr(t.bg)));
        s.theme.set(t);
    });

    let child = |class: *const u16, text: *const u16, style: u32, x: i32, y: i32, cw: i32, ch: i32, id: usize| {
        let c = CreateWindowExW(
            0, class, text, WS_CHILD | WS_VISIBLE | style,
            sc(x, dpi), sc(y, dpi), sc(cw, dpi), sc(ch, dpi),
            hwnd, id as _, hinst, null(),
        );
        set_font(c, font);
        theme::allow_dark_for_window(c, t.dark);
        SetWindowTheme(c, if t.dark { w!("DarkMode_Explorer") } else { w!("Explorer") }, null());
        c
    };
    child(w!("STATIC"), w!("Từ điển được tra theo thứ tự từ trên xuống. Bỏ chọn để tắt, F2 để đổi tên."), 0, 16, 14, 468, 20, 0);
    let list = child(
        w!("SysListView32"), null(),
        WS_TABSTOP | LVS_REPORT | LVS_SINGLESEL | LVS_SHOWSELALWAYS | LVS_EDITLABELS | LVS_NOCOLUMNHEADER,
        20, 44, 460, 262, ID_LIST,
    );
    let status = child(w!("STATIC"), w!(""), 0x80 /* SS_NOPREFIX */ | 0x4000 /* SS_ENDELLIPSIS */, 16, 320, 468, 20, 0);
    child(w!("BUTTON"), w!("Thêm từ điển…"), WS_TABSTOP, 16, 348, 120, 32, ID_ADD);
    child(w!("BUTTON"), w!("Xoá"), WS_TABSTOP, 144, 348, 80, 32, ID_REMOVE);
    child(w!("BUTTON"), w!("Lên"), WS_TABSTOP, 232, 348, 70, 32, ID_UP);
    child(w!("BUTTON"), w!("Xuống"), WS_TABSTOP, 310, 348, 70, 32, ID_DOWN);
    child(w!("BUTTON"), w!("Đóng"), WS_TABSTOP, 404, 348, 80, 32, ID_CLOSE);

    let ex_style = LVS_EX_CHECKBOXES | LVS_EX_FULLROWSELECT | LVS_EX_LABELTIP | LVS_EX_DOUBLEBUFFER;
    SendMessageW(list, LVM_SETEXTENDEDLISTVIEWSTYLE, ex_style as usize, ex_style);
    SendMessageW(list, LVM_SETBKCOLOR, 0, cr(t.field) as LPARAM);
    SendMessageW(list, LVM_SETTEXTBKCOLOR, 0, cr(t.field) as LPARAM);
    SendMessageW(list, LVM_SETTEXTCOLOR, 0, cr(t.text) as LPARAM);
    let count_w = sc(110, dpi);
    for (i, (width, right)) in [(sc(460, dpi) - count_w - sc(24, dpi), false), (count_w, true)].into_iter().enumerate() {
        let mut col: LVCOLUMNW = zeroed();
        col.mask = 0x1 | 0x2 | 0x8; // LVCF_FMT | LVCF_WIDTH | LVCF_SUBITEM
        col.fmt = if right { 1 } else { 0 };
        col.cx = width;
        col.iSubItem = i as i32;
        SendMessageW(list, LVM_INSERTCOLUMNW, i, &col as *const _ as LPARAM);
    }
    m(|s| {
        s.list.set(list);
        s.status.set(status);
    });
    fill(Some(0));
    update_buttons();
    force_foreground(hwnd, list);
}

/// Mở cửa sổ và import ngay các file (dùng cho `evdict.exe --import <file>`).
pub unsafe fn open_and_import(paths: Vec<PathBuf>) {
    open();
    start_import(paths);
}

fn group_digits(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push('.');
        }
        out.push(c);
    }
    out
}

unsafe fn selected() -> Option<usize> {
    let r = SendMessageW(m(|s| s.list.get()), LVM_GETNEXTITEM, usize::MAX, LVNI_SELECTED as LPARAM);
    (r >= 0).then_some(r as usize)
}

unsafe fn set_item_state(list: HWND, i: usize, state: u32, mask: u32) {
    let mut it: LVITEMW = zeroed();
    it.state = state;
    it.stateMask = mask;
    SendMessageW(list, LVM_SETITEMSTATE, i, &it as *const _ as LPARAM);
}

/// Đổ lại danh sách từ thư viện.
unsafe fn fill(select: Option<usize>) {
    let list = m(|s| s.list.get());
    m(|s| s.filling.set(true));
    SendMessageW(list, LVM_DELETEALLITEMS, 0, 0);
    let rows: Vec<(String, String, bool)> = library::with(|items| {
        let rows = items
            .iter()
            .map(|i| {
                let mut name = i.display_name();
                if i.file == BUILTIN {
                    name.push_str(" (có sẵn)");
                }
                let count = i.dict().map(|d| format!("{} mục", group_digits(d.len()))).unwrap_or_else(|| "lỗi file".into());
                (name, count, i.enabled)
            })
            .collect();
        (rows, false)
    });
    for (i, (name, count, enabled)) in rows.iter().enumerate() {
        let mut text = wide(name);
        let mut it: LVITEMW = zeroed();
        it.mask = LVIF_TEXT;
        it.iItem = i as i32;
        it.pszText = text.as_mut_ptr();
        SendMessageW(list, LVM_INSERTITEMW, 0, &it as *const _ as LPARAM);
        let mut text = wide(count);
        it.iSubItem = 1;
        it.pszText = text.as_mut_ptr();
        SendMessageW(list, LVM_SETITEMTEXTW, i, &it as *const _ as LPARAM);
        set_item_state(list, i, (if *enabled { 2 } else { 1 }) << 12, LVIS_STATEIMAGEMASK);
    }
    if let Some(sel) = select.filter(|&s| s < rows.len()) {
        set_item_state(list, sel, LVIS_SELECTED | LVIS_FOCUSED, LVIS_SELECTED | LVIS_FOCUSED);
        SendMessageW(list, LVM_ENSUREVISIBLE, sel, 0);
    }
    m(|s| s.filling.set(false));
}

unsafe fn update_buttons() {
    let hwnd = m(|s| s.hwnd.get());
    let busy = m(|s| s.busy.get());
    let sel = selected();
    let n = library::with(|l| (l.len(), false));
    let builtin = sel.is_some_and(|i| library::with(|l| (l.get(i).is_some_and(|x| x.file == BUILTIN), false)));
    let enable = |id: usize, on: bool| {
        EnableWindow(GetDlgItem(hwnd, id as i32), on as i32);
    };
    enable(ID_ADD, !busy);
    enable(ID_REMOVE, !busy && sel.is_some() && !builtin);
    enable(ID_UP, !busy && sel.is_some_and(|i| i > 0));
    enable(ID_DOWN, !busy && sel.is_some_and(|i| i + 1 < n));
}

unsafe fn set_status(text: &str) {
    SetWindowTextW(m(|s| s.status.get()), wide(text).as_ptr());
}

unsafe fn move_selected(up: bool) {
    let Some(i) = selected() else { return };
    let j = if up { i.checked_sub(1) } else { Some(i + 1) };
    let moved = library::with(|l| match j {
        Some(j) if j < l.len() => {
            l.swap(i, j);
            (Some(j), true)
        }
        _ => (None, false),
    });
    if let Some(j) = moved {
        fill(Some(j));
        update_buttons();
    }
}

unsafe fn remove_selected() {
    let hwnd = m(|s| s.hwnd.get());
    let Some(i) = selected() else { return };
    let name = library::with(|l| (l.get(i).map(|x| x.display_name()), false)).unwrap_or_default();
    let q = wide(&format!("Xoá từ điển \u{201C}{name}\u{201D}? File dữ liệu của nó sẽ bị xoá."));
    if MessageBoxW(hwnd, q.as_ptr(), w!("Quản lý từ điển"), MB_YESNO | MB_ICONQUESTION) != IDYES {
        return;
    }
    match library::remove(i) {
        Ok(()) => set_status(&format!("Đã xoá \u{201C}{name}\u{201D}.")),
        Err(e) => set_status(&e),
    }
    fill(Some(i.saturating_sub(1)));
    update_buttons();
}

/// Hộp thoại chọn file (cho phép chọn nhiều file).
unsafe fn pick_files(owner: HWND) -> Vec<PathBuf> {
    let filter: Vec<u16> = "Từ điển StarDict (*.zip, *.ifo, *.tar.gz, *.tar.bz2)\0*.zip;*.ifo;*.tar.gz;*.tgz;*.tar.bz2;*.tbz2;*.tbz;*.tar\0Tất cả file\0*.*\0\0"
        .encode_utf16()
        .collect();
    let mut buf = vec![0u16; 32 * 1024];
    let mut ofn: OPENFILENAMEW = zeroed();
    ofn.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = owner;
    ofn.lpstrFilter = filter.as_ptr();
    ofn.lpstrFile = buf.as_mut_ptr();
    ofn.nMaxFile = buf.len() as u32;
    ofn.lpstrTitle = w!("Chọn file từ điển StarDict");
    // OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_ALLOWMULTISELECT | OFN_HIDEREADONLY | OFN_NOCHANGEDIR
    ofn.Flags = 0x80000 | 0x1000 | 0x200 | 0x4 | 0x8;
    if GetOpenFileNameW(&mut ofn) == 0 {
        return Vec::new();
    }
    // Một file: "đường dẫn\0\0"; nhiều file: "thư mục\0tên1\0tên2\0\0".
    let parts: Vec<String> = buf
        .split(|&c| c == 0)
        .take_while(|s| !s.is_empty())
        .map(String::from_utf16_lossy)
        .collect();
    match parts.as_slice() {
        [] => Vec::new(),
        [one] => vec![PathBuf::from(one)],
        [dir, files @ ..] => files.iter().map(|f| PathBuf::from(dir).join(f)).collect(),
    }
}

unsafe fn start_import(paths: Vec<PathBuf>) {
    if paths.is_empty() || m(|s| s.busy.get()) {
        return;
    }
    m(|s| s.busy.set(true));
    update_buttons();
    set_status("Đang thêm từ điển… 0%");
    let main = app(|a| a.main.get()) as isize;
    std::thread::spawn(move || {
        let main = main as HWND;
        let n = paths.len() as u32;
        let mut done: Vec<Imported> = Vec::new();
        let mut errors: Vec<String> = Vec::new();
        let mut last = u32::MAX;
        for (i, path) in paths.iter().enumerate() {
            let mut progress = |p: u32| {
                let total = (i as u32 * 100 + p) / n;
                if total != last {
                    last = total;
                    unsafe { PostMessageW(main, WM_APP_IMPORT_PROGRESS, total as usize, 0) };
                }
            };
            match library::import_file(path, &mut progress) {
                Ok(v) => done.extend(v),
                Err(e) => {
                    let name = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                    errors.push(format!("{name}: {e}"));
                }
            }
        }
        let result = Box::into_raw(Box::new((done, errors)));
        if unsafe { PostMessageW(main, WM_APP_IMPORT_DONE, 0, result as LPARAM) } == 0 {
            drop(unsafe { Box::from_raw(result) });
        }
    });
}

/// Cửa sổ chính chuyển tiếp tiến độ import về đây.
pub unsafe fn on_progress(pct: usize) {
    if !m(|s| s.hwnd.get()).is_null() {
        set_status(&format!("Đang thêm từ điển… {pct}%"));
    }
}

/// Cửa sổ chính chuyển tiếp kết quả import về đây (lp = Box<(Vec<Imported>, Vec<String>)>).
pub unsafe fn on_done(lp: LPARAM) {
    let (done, errors) = *Box::from_raw(lp as *mut (Vec<Imported>, Vec<String>));
    library::add(&done.iter().map(|d| d.file.clone()).collect::<Vec<_>>());
    m(|s| s.busy.set(false));
    if m(|s| s.hwnd.get()).is_null() {
        return;
    }
    let mut msg: Vec<String> = done
        .iter()
        .map(|d| format!("\u{201C}{}\u{201D} ({} mục)", d.name, group_digits(d.entries)))
        .collect();
    let mut text = if msg.is_empty() { String::new() } else { format!("Đã thêm {}.", msg.join(", ")) };
    if !errors.is_empty() {
        msg.clear();
        text = format!("{text} Lỗi: {}", errors.join("; ")).trim().to_string();
    }
    set_status(&text);
    let n = library::with(|l| (l.len(), false));
    fill(Some(n.saturating_sub(1)));
    update_buttons();
}

pub unsafe fn handle_key(msg: &MSG) -> bool {
    let (hwnd, list) = m(|s| (s.hwnd.get(), s.list.get()));
    if hwnd.is_null() || (msg.hwnd != hwnd && IsChild(hwnd, msg.hwnd) == 0) {
        return false;
    }
    // Đang sửa tên (ô nhập con của danh sách): để ô nhập tự xử lý Enter/Esc.
    if GetParent(msg.hwnd) == list {
        return false;
    }
    match msg.wParam {
        0x1B => {
            DestroyWindow(hwnd);
            true
        }
        0x71 if msg.hwnd == list => {
            if let Some(i) = selected() {
                SendMessageW(list, LVM_EDITLABELW, i, 0);
            }
            true
        }
        0x2E if msg.hwnd == list => {
            remove_selected();
            true
        }
        _ => false,
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let t = m(|s| s.theme.get());
            let dpi = GetDpiForSystem();
            draw::buffered_paint(
                hwnd,
                t.bg,
                |buf, w, h| {
                    // Khung bo góc quanh danh sách.
                    let r = (sc(16, dpi), sc(40, dpi), sc(468, dpi), sc(270, dpi));
                    let radius = sc(6, dpi) as f32;
                    draw::fill_round_rect(buf, w, h, r, radius, t.field_border);
                    draw::fill_round_rect(buf, w, h, (r.0 + 1, r.1 + 1, r.2 - 2, r.3 - 2), radius - 1.0, t.field);
                },
                |_| {},
            );
            0
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
            let t = m(|s| s.theme.get());
            SetTextColor(wp as HDC, cr(t.text));
            SetBkColor(wp as HDC, cr(t.bg));
            m(|s| s.brush.get()) as LRESULT
        }
        WM_COMMAND => {
            match wp & 0xFFFF {
                ID_ADD => {
                    let files = pick_files(hwnd);
                    start_import(files);
                }
                ID_REMOVE => remove_selected(),
                ID_UP => move_selected(true),
                ID_DOWN => move_selected(false),
                ID_CLOSE => {
                    DestroyWindow(hwnd);
                }
                _ => {}
            }
            0
        }
        WM_NOTIFY => {
            let hdr = &*(lp as *const NMHDR);
            if hdr.idFrom != ID_LIST {
                return 0;
            }
            match hdr.code {
                LVN_ITEMCHANGED => {
                    let nm = &*(lp as *const NMLISTVIEW);
                    let toggled = nm.uChanged & LVIF_STATE != 0
                        && (nm.uNewState ^ nm.uOldState) & LVIS_STATEIMAGEMASK != 0
                        && nm.uOldState & LVIS_STATEIMAGEMASK != 0;
                    if toggled && !m(|s| s.filling.get()) {
                        let on = (nm.uNewState & LVIS_STATEIMAGEMASK) >> 12 == 2;
                        let i = nm.iItem as usize;
                        library::with(|l| {
                            if let Some(x) = l.get_mut(i) {
                                x.enabled = on;
                            }
                            ((), true)
                        });
                    }
                    if (nm.uNewState ^ nm.uOldState) & LVIS_SELECTED != 0 {
                        update_buttons();
                    }
                    0
                }
                LVN_ENDLABELEDITW => {
                    let nm = &*(lp as *const NMLVDISPINFOW);
                    if !nm.item.pszText.is_null() {
                        let mut len = 0;
                        while *nm.item.pszText.add(len) != 0 {
                            len += 1;
                        }
                        let title = String::from_utf16_lossy(std::slice::from_raw_parts(nm.item.pszText, len))
                            .replace(" (có sẵn)", "")
                            .trim()
                            .to_string();
                        let i = nm.item.iItem as usize;
                        library::with(|l| {
                            if let Some(x) = l.get_mut(i) {
                                x.title = title;
                            }
                            ((), true)
                        });
                        fill(Some(i));
                    }
                    0 // tự đổ lại danh sách nên không nhận chữ mới trực tiếp
                }
                _ => 0,
            }
        }
        WM_DESTROY => {
            m(|s| {
                s.hwnd.set(null_mut());
                s.list.set(null_mut());
                s.status.set(null_mut());
                DeleteObject(s.font.replace(null_mut()));
                DeleteObject(s.brush.replace(null_mut()));
            });
            trim_memory();
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
