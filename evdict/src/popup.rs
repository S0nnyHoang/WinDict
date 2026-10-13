//! Cửa sổ nhỏ hiện cạnh con trỏ: ô tìm kiếm ở trên, nghĩa (RichEdit) ở dưới.
//! Giao diện theo Windows 11 (góc bo, ô nhập kiểu Fluent), theo theme sáng/tối
//! của hệ thống và tự co giãn theo nội dung. Cửa sổ bị huỷ khi đóng để không
//! giữ tài nguyên lúc nghỉ.

use std::cell::{Cell, RefCell};
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};

use windows_sys::core::w;
use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{
    DwmFlush, DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE,
};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
use windows_sys::Win32::UI::Controls::SetWindowTheme;
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, GetKeyState};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::dict::{self, Lookup};
use crate::library;
use crate::render;
use crate::draw;
use crate::theme::{self, cr, Theme};
use crate::util::*;
use crate::app;

pub const HINT_TYPE: &str = "Gõ hoặc dán từ tiếng Anh vào ô phía trên.";
pub const HINT_NOSEL: &str = "Không lấy được chữ đang chọn. Hãy bôi đen một từ rồi nhấn phím tắt, hoặc gõ từ vào ô phía trên.";

const ES_MULTILINE: u32 = 0x4;
const ES_AUTOVSCROLL: u32 = 0x40;
const ES_AUTOHSCROLL: u32 = 0x80;
const ES_READONLY: u32 = 0x800;
const EM_SETTEXTEX: u32 = WM_USER + 97;
const EM_SETBKGNDCOLOR: u32 = WM_USER + 67;
const EM_REQUESTRESIZE: u32 = WM_USER + 65;
const EM_SETEVENTMASK: u32 = WM_USER + 69;
const ENM_REQUESTRESIZE: isize = 0x40000;
const EN_REQUESTRESIZE: u32 = 0x701;
const EM_SETCUEBANNER: u32 = 0x1501;
const EN_SETFOCUS: usize = 0x100;
const EN_KILLFOCUS: usize = 0x200;
const EN_CHANGE: usize = 0x300;
const TIMER_SEARCH: usize = 1;
const TYPING_DELAY_MS: u32 = 120;

// Kích thước logic (96 DPI).
const WIDTH: i32 = 440;
const PAD: i32 = 12;
const FIELD_H: i32 = 36;
const GAP: i32 = 8;
const MIN_H: i32 = 120;
const MAX_H: i32 = 480;
const TAB_H: i32 = 32;
const TAB_PAD: i32 = 12;

#[repr(C)]
struct SetTextEx {
    flags: u32,
    codepage: u32,
}

#[repr(C)]
struct NmHdr {
    hwnd_from: HWND,
    id_from: usize,
    code: u32,
}

#[repr(C)]
struct ReqResize {
    nmhdr: NmHdr,
    rc: RECT,
}

/// Trạng thái riêng của popup đang mở.
struct State {
    theme: Cell<Theme>,
    edit_brush: Cell<HBRUSH>,
    icon_font: Cell<HFONT>,
    line_h: Cell<i32>,
    content_h: Cell<i32>,
    rounded: Cell<bool>,
    /// Chuỗi đã tra gần nhất, để không tra lại khi nội dung ô tìm kiếm không đổi.
    last_query: RefCell<String>,
    /// Độ dài ô tìm kiếm lần trước, để nhận biết dán (thay đổi nhiều ký tự một lúc).
    edit_len: Cell<i32>,
    /// Kết quả theo từng từ điển: (tên, RTF). Hiện hàng tab khi có từ 2 kết quả.
    tabs: RefCell<Vec<(String, String)>>,
    active: Cell<usize>,
    tab_rects: RefCell<Vec<RECT>>,
}

thread_local! {
    static ST: State = State {
        theme: Cell::new(theme::LIGHT),
        edit_brush: Cell::new(null_mut()),
        icon_font: Cell::new(null_mut()),
        line_h: Cell::new(20),
        content_h: Cell::new(0),
        rounded: Cell::new(false),
        last_query: RefCell::new(String::new()),
        edit_len: Cell::new(0),
        tabs: RefCell::new(Vec::new()),
        active: Cell::new(0),
        tab_rects: RefCell::new(Vec::new()),
    };
}

fn st<R>(f: impl FnOnce(&State) -> R) -> R {
    ST.with(f)
}

static RICHEDIT_LOADED: AtomicBool = AtomicBool::new(false);

pub unsafe fn register_class(hinst: HINSTANCE) {
    let mut wc: WNDCLASSEXW = zeroed();
    wc.cbSize = size_of::<WNDCLASSEXW>() as u32;
    wc.lpfnWndProc = Some(wndproc);
    wc.hInstance = hinst;
    wc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
    wc.lpszClassName = w!("EVDictPopup");
    RegisterClassExW(&wc);
}

pub unsafe fn show(text: Option<String>, hint: &str) {
    if !RICHEDIT_LOADED.swap(true, Ordering::Relaxed) {
        LoadLibraryW(w!("Msftedit.dll"));
    }
    close();
    trace("richedit loaded");

    let t = theme::current();
    let fonts = theme::fonts();
    let mut pt: POINT = zeroed();
    GetCursorPos(&mut pt);
    let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
    let (mut dpi, mut dpi_y) = (96u32, 96u32);
    GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dpi, &mut dpi_y);

    // Tạo ẩn với chiều cao tối đa, đổ nội dung, rồi mới co lại và đặt vị trí.
    let hinst = GetModuleHandleW(null());
    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
        w!("EVDictPopup"),
        w!("Từ điển Anh-Việt"),
        WS_POPUP | WS_CLIPCHILDREN,
        pt.x, pt.y, sc(WIDTH, dpi), sc(MAX_H, dpi),
        null_mut(), null_mut(), hinst, null(),
    );
    let corner: u32 = 2; // DWMWCP_ROUND
    let rounded = DwmSetWindowAttribute(
        hwnd, DWMWA_WINDOW_CORNER_PREFERENCE as u32, &corner as *const u32 as _, 4,
    ) >= 0;
    let border = cr(t.window_border);
    DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR as u32, &border as *const u32 as _, 4);

    let edit = CreateWindowExW(
        0, w!("EDIT"), null(), WS_CHILD | WS_VISIBLE | ES_AUTOHSCROLL,
        0, 0, 0, 0, hwnd, 100 as _, hinst, null(),
    );
    let rich = CreateWindowExW(
        0, w!("RICHEDIT50W"), null(),
        WS_CHILD | WS_VISIBLE | WS_VSCROLL | ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL,
        0, 0, 0, 0, hwnd, 101 as _, hinst, null(),
    );

    let font = make_font(fonts.text, 11, dpi);
    set_font(edit, font);
    SendMessageW(edit, EM_SETCUEBANNER, 1, wide("Nhập từ tiếng Anh…").as_ptr() as LPARAM);
    let line_h = {
        let dc = GetDC(edit);
        let old = SelectObject(dc, font);
        let mut tm: TEXTMETRICW = zeroed();
        GetTextMetricsW(dc, &mut tm);
        SelectObject(dc, old);
        ReleaseDC(edit, dc);
        tm.tmHeight
    };

    SendMessageW(rich, EM_SETBKGNDCOLOR, 0, cr(t.bg) as LPARAM);
    SendMessageW(rich, EM_SETEVENTMASK, 0, ENM_REQUESTRESIZE);
    theme::allow_dark_for_window(rich, t.dark);
    SetWindowTheme(rich, if t.dark { w!("DarkMode_Explorer") } else { w!("Explorer") }, null());

    st(|s| {
        s.theme.set(t);
        s.edit_brush.set(CreateSolidBrush(cr(t.field)));
        s.icon_font.set(make_font(fonts.icons, 11, dpi));
        s.line_h.set(line_h);
        s.content_h.set(0);
        s.rounded.set(rounded);
        s.last_query.borrow_mut().clear();
        s.edit_len.set(0);
        s.tabs.borrow_mut().clear();
        s.tab_rects.borrow_mut().clear();
        s.active.set(0);
    });
    app(|a| {
        a.popup.set(hwnd);
        a.edit.set(edit);
        a.rich.set(rich);
        a.popup_font.set(font);
        a.popup_dpi.set(dpi);
    });
    layout(hwnd);
    trace("popup created");

    match text {
        Some(t) => {
            search(&t);
            SetWindowTextW(edit, wide(&dict::clean(&t)).as_ptr());
        }
        None => {
            st(|s| s.tabs.borrow_mut().clear());
            set_rtf(rich, &render::message(&t, hint, &[]));
        }
    }
    fit(hwnd, Some(pt));
    trace("content rendered");

    force_foreground(hwnd, edit);
    SendMessageW(edit, EM_SETSEL, 0, -1);
}

pub unsafe fn close() {
    let h = app(|a| a.popup.get());
    if !h.is_null() {
        DestroyWindow(h);
        trim_memory();
    }
}

/// Phím trong popup: Esc đóng, Enter tra từ trong ô tìm kiếm,
/// mũi tên/PageUp/PageDown cuộn phần nghĩa.
pub unsafe fn handle_key(msg: &MSG) -> bool {
    let (popup, edit, rich) = app(|a| (a.popup.get(), a.edit.get(), a.rich.get()));
    if popup.is_null() || (msg.hwnd != popup && IsChild(popup, msg.hwnd) == 0) {
        return false;
    }
    match msg.wParam {
        0x1B => {
            close();
            true
        }
        // Tab / Shift+Tab (hoặc Ctrl+Tab): chuyển tab từ điển.
        0x09 if tab_count() >= 2 => {
            let n = tab_count();
            let back = GetKeyState(0x10) < 0;
            let cur = st(|s| s.active.get());
            select_tab(if back { (cur + n - 1) % n } else { (cur + 1) % n });
            true
        }
        0x0D if msg.hwnd == edit => {
            KillTimer(popup, TIMER_SEARCH);
            search(&window_text(edit));
            fit(popup, None);
            SendMessageW(edit, EM_SETSEL, 0, -1);
            true
        }
        0x21 | 0x22 | 0x26 | 0x28 if msg.hwnd == edit => {
            let dpi = app(|a| a.popup_dpi.get());
            let mut rc: RECT = zeroed();
            GetClientRect(rich, &mut rc);
            let page = (rc.bottom as f64 * 0.85).max(1.0);
            let line = sc(LINE_STEP, dpi) as f64;
            smooth_scroll_by(match msg.wParam {
                0x21 => -page,
                0x22 => page,
                0x26 => -line,
                _ => line,
            });
            true
        }
        _ => false,
    }
}

// ---------- Cuộn mượt ----------
// RichEdit tự cuộn bằng cách nhảy 3 dòng mỗi nấc chuột. Ở đây ta tự trượt vị trí
// cuộn tới đích theo thời gian (ease-out), mỗi khung hình đồng bộ với tần số quét
// của màn hình qua DwmFlush.

const EM_GETSCROLLPOS: u32 = WM_USER + 221;
const EM_SETSCROLLPOS: u32 = WM_USER + 222;
const SCROLL_MS: f64 = 180.0;
/// Một "dòng" khi cuộn (pixel logic).
const LINE_STEP: i32 = 20;

struct Anim {
    running: Cell<bool>,
    from: Cell<f64>,
    target: Cell<f64>,
    start: Cell<Option<std::time::Instant>>,
}

thread_local! {
    static ANIM: Anim = const {
        Anim { running: Cell::new(false), from: Cell::new(0.0), target: Cell::new(0.0), start: Cell::new(None) }
    };
}

unsafe fn scroll_pos(rich: HWND) -> f64 {
    let mut pt: POINT = zeroed();
    SendMessageW(rich, EM_GETSCROLLPOS, 0, &mut pt as *mut POINT as LPARAM);
    pt.y as f64
}

unsafe fn max_scroll(rich: HWND) -> f64 {
    let mut rc: RECT = zeroed();
    GetClientRect(rich, &mut rc);
    (st(|s| s.content_h.get()) - rc.bottom).max(0) as f64
}

/// Cuộn chuột trên popup: trả về true nếu đã xử lý.
pub unsafe fn handle_wheel(msg: &MSG) -> bool {
    let (popup, dpi) = app(|a| (a.popup.get(), a.popup_dpi.get()));
    if popup.is_null() || (msg.hwnd != popup && IsChild(popup, msg.hwnd) == 0) {
        return false;
    }
    let delta = ((msg.wParam >> 16) & 0xFFFF) as u16 as i16 as f64;
    let mut lines: u32 = 3;
    SystemParametersInfoW(SPI_GETWHEELSCROLLLINES, 0, &mut lines as *mut u32 as _, 0);
    let lines = if lines == u32::MAX { 3 } else { lines.max(1) }; // u32::MAX = "cuộn cả trang"
    smooth_scroll_by(-delta / 120.0 * lines as f64 * sc(LINE_STEP, dpi) as f64);
    true
}

unsafe fn smooth_scroll_by(dy: f64) {
    let rich = app(|a| a.rich.get());
    if rich.is_null() {
        return;
    }
    let now = std::time::Instant::now();
    let running = ANIM.with(|a| a.running.get());
    // Đang trượt thì cộng dồn vào đích cũ; nếu không thì tính từ vị trí hiện tại
    // (người dùng có thể vừa kéo thanh cuộn).
    let base = if running { ANIM.with(|a| a.target.get()) } else { scroll_pos(rich) };
    let target = (base + dy).clamp(0.0, max_scroll(rich));
    let current = scroll_pos(rich);
    ANIM.with(|a| {
        a.from.set(current);
        a.target.set(target);
        a.start.set(Some(now));
    });
    if !running {
        run_animation(rich);
    }
}

/// Vòng lặp hoạt ảnh: mỗi vòng đặt vị trí cuộn, vẽ ngay, rồi chờ khung hình kế tiếp
/// của màn hình. Thông điệp mới (thêm nấc cuộn, phím, đóng popup) vẫn được xử lý trong lúc trượt.
unsafe fn run_animation(rich: HWND) {
    ANIM.with(|a| a.running.set(true));
    let tracing = std::env::var_os("EVDICT_TRACE").is_some();
    let mut frames: Vec<f64> = Vec::new();
    let mut snapped = 0; // số khung hình RichEdit không đặt đúng pixel yêu cầu
    let mut last = std::time::Instant::now();
    loop {
        let (from, target, start) = ANIM.with(|a| (a.from.get(), a.target.get(), a.start.get()));
        let t = start.map_or(1.0, |s| (s.elapsed().as_secs_f64() * 1000.0 / SCROLL_MS).min(1.0));
        let ease = 1.0 - (1.0 - t).powi(3);
        let y = from + (target - from) * ease;
        let mut pt = POINT { x: 0, y: y.round() as i32 };
        SendMessageW(rich, EM_SETSCROLLPOS, 0, &mut pt as *mut POINT as LPARAM);
        UpdateWindow(rich);
        if tracing && (scroll_pos(rich) - pt.y as f64).abs() > 1.0 {
            snapped += 1;
        }
        if t >= 1.0 {
            break;
        }
        if DwmFlush() < 0 {
            windows_sys::Win32::System::Threading::Sleep(8);
        }
        if tracing {
            frames.push(last.elapsed().as_secs_f64() * 1000.0);
            last = std::time::Instant::now();
        }
        let mut msg: MSG = zeroed();
        while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
            if msg.message == WM_QUIT {
                PostQuitMessage(msg.wParam as i32);
                ANIM.with(|a| a.running.set(false));
                return;
            }
            crate::dispatch(&msg);
        }
        if app(|a| a.rich.get()) != rich {
            break; // Popup đã đóng.
        }
    }
    ANIM.with(|a| a.running.set(false));
    if tracing && !frames.is_empty() {
        let avg = frames.iter().sum::<f64>() / frames.len() as f64;
        let max = frames.iter().cloned().fold(0.0, f64::max);
        trace_line(&format!(
            "scroll: {} frames, avg {avg:.1} ms, max {max:.1} ms, snapped {snapped}, end y={}",
            frames.len(),
            scroll_pos(rich)
        ));
    }
}

/// Vị trí ô tìm kiếm trong popup (pixel thật).
fn field_rect(width: i32, dpi: u32) -> (i32, i32, i32, i32) {
    (sc(PAD, dpi), sc(PAD, dpi), width - 2 * sc(PAD, dpi), sc(FIELD_H, dpi))
}

unsafe fn layout(hwnd: HWND) {
    let (edit, rich, dpi) = app(|a| (a.edit.get(), a.rich.get(), a.popup_dpi.get()));
    if edit.is_null() {
        return;
    }
    let mut rc: RECT = zeroed();
    GetClientRect(hwnd, &mut rc);
    let (fx, fy, fw, fh) = field_rect(rc.right, dpi);
    let line_h = st(|s| s.line_h.get());
    let text_x = fx + sc(36, dpi);
    MoveWindow(edit, text_x, fy + (fh - line_h) / 2, fx + fw - sc(10, dpi) - text_x, line_h, 1);
    let top = content_top(dpi);
    MoveWindow(rich, fx - sc(2, dpi), top, rc.right - fx + sc(2, dpi), rc.bottom - top - sc(6, dpi), 1);
    compute_tab_rects(hwnd, rc.right, dpi);
}

/// Toạ độ y của hàng tab (ngay dưới ô tìm kiếm).
fn tabs_top(dpi: u32) -> i32 {
    sc(PAD + FIELD_H + 4, dpi)
}

/// Toạ độ y bắt đầu phần nghĩa (dưới hàng tab nếu có).
fn content_top(dpi: u32) -> i32 {
    if tab_count() >= 2 {
        tabs_top(dpi) + sc(TAB_H + 6, dpi)
    } else {
        sc(PAD + FIELD_H + GAP, dpi)
    }
}

/// Tính vị trí từng tab theo độ rộng tên; tên dài bị cắt bớt "…" khi vẽ.
unsafe fn compute_tab_rects(hwnd: HWND, width: i32, dpi: u32) {
    let names: Vec<String> = st(|s| s.tabs.borrow().iter().map(|t| t.0.clone()).collect());
    let mut rects = Vec::new();
    if names.len() >= 2 {
        let font = app(|a| a.popup_font.get());
        let dc = GetDC(hwnd);
        let old = SelectObject(dc, font);
        let pad = sc(TAB_PAD, dpi);
        let avail = width - 2 * sc(PAD, dpi);
        let max_w = (avail / names.len() as i32).max(sc(60, dpi));
        let (mut x, top) = (sc(PAD, dpi), tabs_top(dpi));
        for name in &names {
            let s: Vec<u16> = name.encode_utf16().collect();
            let mut size: SIZE = zeroed();
            GetTextExtentPoint32W(dc, s.as_ptr(), s.len() as i32, &mut size);
            let w = (size.cx + 2 * pad).min(max_w);
            rects.push(RECT { left: x, top, right: x + w, bottom: top + sc(TAB_H, dpi) });
            x += w;
        }
        SelectObject(dc, old);
        ReleaseDC(hwnd, dc);
    }
    st(|s| *s.tab_rects.borrow_mut() = rects);
}

fn tab_at(x: i32, y: i32) -> Option<usize> {
    st(|s| {
        s.tab_rects
            .borrow()
            .iter()
            .position(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
    })
}

/// Co giãn chiều cao theo nội dung; `anchor` là vị trí con trỏ khi mới mở.
unsafe fn fit(hwnd: HWND, anchor: Option<POINT>) {
    let dpi = app(|a| a.popup_dpi.get());
    let want = content_top(dpi) + sc(16, dpi) + st(|s| s.content_h.get());
    let h = want.clamp(sc(MIN_H, dpi), sc(MAX_H, dpi));
    let w = sc(WIDTH, dpi);

    let mut cur: RECT = zeroed();
    GetWindowRect(hwnd, &mut cur);
    let mon = match anchor {
        Some(pt) => MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST),
        None => MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
    };
    let mut mi: MONITORINFO = zeroed();
    mi.cbSize = size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(mon, &mut mi);
    let work = mi.rcWork;

    let (mut x, mut y) = match anchor {
        Some(pt) => {
            let x = pt.x + sc(12, dpi);
            let mut y = pt.y + sc(18, dpi);
            if y + h > work.bottom {
                y = pt.y - h - sc(8, dpi);
            }
            (x, y)
        }
        None => (cur.left, cur.top),
    };
    x = x.min(work.right - w).max(work.left);
    y = y.min(work.bottom - h).max(work.top);
    SetWindowPos(hwnd, null_mut(), x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
    let rich = app(|a| a.rich.get());
    SendMessageW(rich, WM_VSCROLL, SB_TOP as usize, 0);
}

unsafe fn paint(hwnd: HWND) {
    let (t, rounded, icon_font) = st(|s| (s.theme.get(), s.rounded.get(), s.icon_font.get()));
    let (dpi, edit) = app(|a| (a.popup_dpi.get(), a.edit.get()));
    let focused = GetFocus() == edit;
    let mut rc: RECT = zeroed();
    GetClientRect(hwnd, &mut rc);
    let (fx, fy, fw, fh) = field_rect(rc.right, dpi);
    let rects = st(|s| s.tab_rects.borrow().clone());
    let names: Vec<String> = st(|s| s.tabs.borrow().iter().map(|t| t.0.clone()).collect());
    let active = st(|s| s.active.get());
    draw::buffered_paint(
        hwnd,
        t.bg,
        |buf, w, h| {
            if !rounded {
                // Windows 10: không có viền DWM, tự vẽ viền 1px.
                draw::fill_round_rect(buf, w, h, (0, 0, w, h), 0.0, t.window_border);
                draw::fill_round_rect(buf, w, h, (1, 1, w - 2, h - 2), 0.0, t.bg);
            }
            draw::field(buf, w, h, (fx, fy, fw, fh), dpi, &t, focused);
            if let (Some(first), Some(r)) = (rects.first(), rects.get(active)) {
                // Đường kẻ dưới hàng tab và gạch chân màu nhấn cho tab đang chọn.
                let base = first.bottom - 1;
                draw::fill_round_rect(buf, w, h, (fx, base, fw, 1), 0.0, t.field_border);
                let pad = sc(TAB_PAD, dpi);
                let bar = sc(3, dpi);
                draw::fill_round_rect(
                    buf, w, h,
                    (r.left + pad, r.bottom - bar, r.right - r.left - 2 * pad, bar),
                    bar as f32 / 2.0,
                    t.accent,
                );
            }
        },
        |dc| {
            SelectObject(dc, icon_font);
            SetTextColor(dc, cr(t.text2));
            let mut icon_rc = RECT { left: fx + sc(4, dpi), top: fy, right: fx + sc(34, dpi), bottom: fy + fh };
            let mut glyph = [0xE721u16]; // Search
            DrawTextW(dc, glyph.as_mut_ptr(), 1, &mut icon_rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
            SelectObject(dc, app(|a| a.popup_font.get()));
            for (i, (r, name)) in rects.iter().zip(&names).enumerate() {
                SetTextColor(dc, cr(if i == active { t.text } else { t.text2 }));
                let pad = sc(TAB_PAD, dpi) - sc(2, dpi);
                let mut tr = RECT { left: r.left + pad, top: r.top, right: r.right - pad, bottom: r.bottom - sc(3, dpi) };
                let mut s: Vec<u16> = name.encode_utf16().collect();
                DrawTextW(dc, s.as_mut_ptr(), s.len() as i32, &mut tr, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
            }
        },
    );
}
/// Tra và hiển thị kết quả cho chuỗi trong ô tìm kiếm.
unsafe fn search(query: &str) {
    let t = st(|s| s.theme.get());
    let tabs = lookup_all(query, &t);
    let first = tabs[0].1.clone();
    st(|s| {
        *s.last_query.borrow_mut() = dict::clean(query);
        *s.tabs.borrow_mut() = tabs;
        s.active.set(0);
    });
    let popup = app(|a| a.popup.get());
    layout(popup);
    InvalidateRect(popup, null(), 0);
    set_rtf(app(|a| a.rich.get()), &first);
}

/// Chuyển sang tab từ điển thứ i.
unsafe fn select_tab(i: usize) {
    let Some(rtf) = st(|s| s.tabs.borrow().get(i).map(|t| t.1.clone())) else { return };
    st(|s| s.active.set(i));
    let popup = app(|a| a.popup.get());
    set_rtf(app(|a| a.rich.get()), &rtf);
    fit(popup, None);
    InvalidateRect(popup, null(), 0);
}

fn tab_count() -> usize {
    st(|s| s.tabs.borrow().len())
}

unsafe fn set_rtf(rich: HWND, rtf: &str) {
    let mut bytes = rtf.as_bytes().to_vec();
    bytes.push(0);
    let st = SetTextEx { flags: 0, codepage: 0 };
    SendMessageW(rich, EM_SETTEXTEX, &st as *const _ as WPARAM, bytes.as_ptr() as LPARAM);
    SendMessageW(rich, EM_REQUESTRESIZE, 0, 0);
}

/// Tra trong mọi từ điển đang bật. Trả về các tab (tên từ điển, RTF); không có kết quả
/// thì một "tab" không tên chứa thông báo.
fn lookup_all(query: &str, t: &Theme) -> Vec<(String, String)> {
    let dicts = library::enabled();
    if dicts.is_empty() {
        let msg = "Chưa có từ điển nào được bật. Mở \u{201C}Quản lý từ điển\u{201D} từ icon ở khay hệ thống.";
        return vec![(String::new(), render::message(t, msg, &[]))];
    }
    // (tên, RTF, khớp chính xác?). Nếu có bộ khớp chính xác thì bỏ các bộ chỉ khớp
    // nhờ đoán dạng gốc/rút gọn cụm từ, ví dụ "running" -> "run" (tiếng Việt: run rẩy)
    // trong từ điển Việt-Anh.
    let mut found: Vec<(String, String, bool)> = Vec::new();
    let mut not_found: Option<(String, Vec<String>)> = None;
    for (name, d) in &dicts {
        match dict::lookup(d, query) {
            Lookup::Found { idx, note } => {
                found.push((name.clone(), render::entry(t, &d.text(idx), note.as_deref()), note.is_none()))
            }
            Lookup::NotFound { query, suggest } => {
                if not_found.as_ref().map_or(true, |n| n.1.is_empty()) {
                    not_found = Some((query, suggest));
                }
            }
        }
    }
    if found.iter().any(|f| f.2) {
        found.retain(|f| f.2);
    }
    let mut tabs: Vec<(String, String)> = found.into_iter().map(|f| (f.0, f.1)).collect();
    if tabs.is_empty() {
        let (q, suggest) = not_found.unwrap_or_default();
        let rtf = if q.is_empty() {
            render::message(t, HINT_TYPE, &[])
        } else {
            render::message(t, &format!("Không tìm thấy \u{201C}{q}\u{201D} trong từ điển."), &suggest)
        };
        tabs.push((String::new(), rtf));
    }
    tabs
}
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_SIZE => {
            layout(hwnd);
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint(hwnd);
            0
        }
        WM_CTLCOLOREDIT => {
            let t = st(|s| s.theme.get());
            let dc = wp as HDC;
            SetTextColor(dc, cr(t.text));
            SetBkColor(dc, cr(t.field));
            st(|s| s.edit_brush.get()) as LRESULT
        }
        WM_COMMAND => {
            match wp >> 16 {
                // Ô tìm kiếm nhận/mất focus: vẽ lại gạch dưới màu nhấn.
                EN_SETFOCUS | EN_KILLFOCUS => {
                    InvalidateRect(hwnd, null(), 0);
                }
                // Nội dung đổi: dán (đổi nhiều ký tự một lúc) thì tra ngay,
                // gõ từng phím thì đợi người dùng dừng tay một chút.
                EN_CHANGE => {
                    let len = GetWindowTextLengthW(lp as HWND);
                    let jump = (len - st(|s| s.edit_len.replace(len))).abs() > 1;
                    SetTimer(hwnd, TIMER_SEARCH, if jump { 0 } else { TYPING_DELAY_MS }, None);
                }
                _ => {}
            }
            0
        }
        WM_TIMER if wp == TIMER_SEARCH => {
            KillTimer(hwnd, TIMER_SEARCH);
            let edit = app(|a| a.edit.get());
            let text = window_text(edit);
            if dict::clean(&text) != st(|s| s.last_query.borrow().clone()) {
                search(&text);
                fit(hwnd, None);
            }
            0
        }
        WM_NOTIFY => {
            let nm = &*(lp as *const NmHdr);
            if nm.code == EN_REQUESTRESIZE {
                let rr = &*(lp as *const ReqResize);
                st(|s| s.content_h.set(rr.rc.bottom - rr.rc.top));
            }
            0
        }
        WM_ACTIVATE => {
            if wp & 0xFFFF == 0 {
                // Mất focus (click ra ngoài) thì đóng.
                PostMessageW(hwnd, WM_CLOSE, 0, 0);
            }
            0
        }
        WM_CLOSE => {
            close();
            0
        }
        WM_LBUTTONDOWN => {
            let (x, y) = ((lp & 0xFFFF) as i16 as i32, ((lp >> 16) & 0xFFFF) as i16 as i32);
            if let Some(i) = tab_at(x, y) {
                select_tab(i);
            }
            0
        }
        WM_SETCURSOR if tab_count() >= 2 => {
            let mut pt: POINT = zeroed();
            GetCursorPos(&mut pt);
            ScreenToClient(hwnd, &mut pt);
            if tab_at(pt.x, pt.y).is_some() {
                SetCursor(LoadCursorW(null_mut(), IDC_HAND));
                return 1;
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_NCHITTEST => {
            // Cho phép kéo cửa sổ bằng phần lề (trừ hàng tab để bấm được).
            let r = DefWindowProcW(hwnd, msg, wp, lp);
            let mut pt = POINT { x: (lp & 0xFFFF) as i16 as i32, y: ((lp >> 16) & 0xFFFF) as i16 as i32 };
            ScreenToClient(hwnd, &mut pt);
            if r == HTCLIENT as LRESULT && tab_at(pt.x, pt.y).is_some() {
                return r;
            }
            if r == HTCLIENT as LRESULT {
                HTCAPTION as LRESULT
            } else {
                r
            }
        }
        WM_DESTROY => {
            if app(|a| a.popup.get()) == hwnd {
                app(|a| {
                    a.popup.set(null_mut());
                    a.edit.set(null_mut());
                    a.rich.set(null_mut());
                    DeleteObject(a.popup_font.replace(null_mut()));
                });
                st(|s| {
                    DeleteObject(s.edit_brush.replace(null_mut()));
                    DeleteObject(s.icon_font.replace(null_mut()));
                });
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
