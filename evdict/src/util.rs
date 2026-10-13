use std::path::PathBuf;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Gdi::{CreateFontIndirectW, HFONT, LOGFONTW};
use windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows_sys::Win32::System::Threading::{
    AttachThreadInput, GetCurrentProcess, GetCurrentThreadId, SetProcessWorkingSetSize,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, SendMessageW, SetForegroundWindow, ShowWindow, SW_SHOWNORMAL,
    WM_SETFONT,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;

pub const EM_SETSEL: u32 = 0xB1;

// Đo thời gian từng bước khi đặt biến môi trường EVDICT_TRACE=1; ghi ra trace.log cạnh exe.
thread_local! {
    static TRACE: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
}

pub fn trace_start() {
    if std::env::var_os("EVDICT_TRACE").is_some() {
        TRACE.with(|t| t.set(Some(std::time::Instant::now())));
        trace("--- hotkey");
    }
}

pub fn trace(label: &str) {
    if let Some(t0) = TRACE.with(|t| t.get()) {
        use std::io::Write;
        let path = exe_dir().join("trace.log");
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{:7.1} ms  {label}", t0.elapsed().as_secs_f64() * 1000.0);
        }
    }
}

/// Ghi một dòng vào trace.log (khi EVDICT_TRACE được đặt), không cần mốc thời gian.
pub fn trace_line(s: &str) {
    if std::env::var_os("EVDICT_TRACE").is_some() {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(exe_dir().join("trace.log")) {
            let _ = writeln!(f, "{s}");
        }
    }
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn exe_path() -> String {
    let mut buf = vec![0u16; 1024];
    let n = unsafe { GetModuleFileNameW(null_mut(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
    String::from_utf16_lossy(&buf[..n])
}

pub fn exe_dir() -> PathBuf {
    PathBuf::from(exe_path())
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default()
}

/// Đổi kích thước logic (96 DPI) sang pixel thật.
pub fn sc(v: i32, dpi: u32) -> i32 {
    v * dpi as i32 / 96
}

pub unsafe fn window_text(h: HWND) -> String {
    let n = GetWindowTextLengthW(h);
    let mut buf = vec![0u16; n as usize + 1];
    let r = GetWindowTextW(h, buf.as_mut_ptr(), n + 1);
    String::from_utf16_lossy(&buf[..r.max(0) as usize])
}

pub unsafe fn make_font(face: &str, pt: i32, dpi: u32) -> HFONT {
    let mut lf: LOGFONTW = std::mem::zeroed();
    lf.lfHeight = -(pt * dpi as i32 / 72);
    lf.lfWeight = 400;
    lf.lfCharSet = 1; // DEFAULT_CHARSET
    lf.lfQuality = 5; // CLEARTYPE_QUALITY
    for (i, c) in face.encode_utf16().take(31).enumerate() {
        lf.lfFaceName[i] = c;
    }
    CreateFontIndirectW(&lf)
}

pub unsafe fn set_font(h: HWND, f: HFONT) {
    SendMessageW(h, WM_SETFONT, f as usize, 1);
}

/// Hiện cửa sổ và đưa lên trước kể cả khi Windows đang chặn đổi cửa sổ foreground:
/// tạm gắn vào luồng nhập liệu của cửa sổ đang active, hiện + đặt focus, rồi tách ra.
pub unsafe fn force_foreground(hwnd: HWND, focus: HWND) {
    let fg = GetForegroundWindow();
    let fg_thread = GetWindowThreadProcessId(fg, null_mut());
    let me = GetCurrentThreadId();
    let attach = fg_thread != 0 && fg_thread != me;
    if attach {
        AttachThreadInput(me, fg_thread, 1);
    }
    ShowWindow(hwnd, SW_SHOWNORMAL);
    BringWindowToTop(hwnd);
    SetForegroundWindow(hwnd);
    SetFocus(focus);
    if attach {
        AttachThreadInput(me, fg_thread, 0);
    }
}

/// Trả lại bộ nhớ không dùng cho hệ điều hành khi ứng dụng đang nghỉ.
pub fn trim_memory() {
    unsafe {
        SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}
