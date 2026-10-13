# EVDict: từ điển Anh–Việt nhẹ cho Windows

Bôi đen một từ tiếng Anh ở bất kỳ ứng dụng nào rồi bấm **Ctrl+Alt+D**: nghĩa tiếng Việt hiện ngay cạnh con trỏ.

## Sử dụng

Chạy `dist\evdict.exe`. Hai file `evdict.exe` và `evdict.dat` phải nằm cùng thư mục. Ứng dụng chạy nền với icon chữ **D** ở khay hệ thống.

- **Tra từ**: bôi đen rồi bấm phím tắt. Gõ hoặc dán từ khác vào ô tìm kiếm là tự tra (dán: tra ngay; gõ: tra khi ngừng gõ ~120 ms).
- **Trong popup**: `Esc` hoặc click ra ngoài để đóng; `↑ ↓ PgUp PgDn` để cuộn; kéo lề để di chuyển.
- **Click trái icon**: mở ô tra từ.
- **Click phải icon**:
  - Đổi phím tắt
  - Bật/tắt *Khởi động cùng Windows*
  - Thoát
- **Dòng lệnh**: `evdict.exe hello` mở popup tra từ "hello"; `evdict.exe --settings` mở hộp thoại đổi phím tắt.
- **Giao diện** tự theo chế độ sáng/tối của Windows. Popup tự co giãn theo độ dài nghĩa.

## Thêm từ điển

Click phải icon khay → **Quản lý từ điển...**:

- **Thêm từ điển…**: chọn file StarDict `.zip`, `.tar.gz`, `.tar.bz2`, `.tar` hoặc `.ifo` (cần các file `.idx`, `.dict`/`.dict.dz` cùng tên bên cạnh). Một file nén chứa nhiều bộ sẽ được thêm hết. Nội dung chữ thường, HTML, XDXF, Pango và từ đồng nghĩa (`.syn`) đều được hỗ trợ.
- **Ô chọn**: bật/tắt. **Lên/Xuống**: thứ tự ưu tiên (thứ tự tab). **F2**: đổi tên. **Xoá**/phím Delete: gỡ và xoá file.
- Khi tra, mỗi từ điển có kết quả là một tab dưới ô tìm kiếm (chuyển bằng chuột hoặc phím Tab). Nếu có từ điển khớp chính xác thì các từ điển chỉ khớp nhờ đoán dạng gốc sẽ bị ẩn.
- Dòng lệnh: `evdict.exe --dicts` mở cửa sổ này; `evdict.exe --import <file>` thêm từ điển.

Từ điển đã thêm được chuyển sang định dạng nén riêng và lưu ở `%APPDATA%\EVDict\dicts`, danh sách ở `%APPDATA%\EVDict\dicts.txt`. Đặt biến môi trường `EVDICT_HOME` để dùng thư mục khác (ví dụ chạy di động từ USB).

Cấu hình lưu tại `HKCU\Software\EVDict`. Khởi động cùng Windows dùng khoá `HKCU\...\CurrentVersion\Run`.

## Tải về

Tải bản mới nhất ở trang **[Releases](https://github.com/S0nnyHoang/WinDict/releases/latest)**:

- **EVDict-Setup-x.y.z.exe**: trình cài đặt, không cần quyền admin, có shortcut Start Menu và tuỳ chọn khởi động cùng Windows. Gỡ cài đặt trong Settings → Apps.
- **EVDict-Portable-x.y.z.zip**: giải nén ra đâu cũng được rồi chạy `evdict.exe`. Từ điển thêm vào lưu trong thư mục `data\` bên cạnh (do có file `portable.txt`).

## Build

Cần Rust (toolchain `stable-x86_64-pc-windows-gnu`).

```bash
powershell -ExecutionPolicy Bypass -File build.ps1
```

`build.ps1` biên dịch ứng dụng và chuyển dữ liệu StarDict trong `data\raw` thành `dist\evdict.dat`. Để kiểm tra logic tra cứu: `cargo test --release -p evdict`.

## Cấu trúc

| File | Vai trò |
|---|---|
| `evdict/src/main.rs` | Khởi động, icon khay, menu, phím tắt |
| `evdict/src/clip.rs` | Lấy chữ đang chọn (Ctrl+C giả lập) và trả lại clipboard cũ |
| `evdict/src/dict.rs` | Đọc một từ điển `.dat` qua memory-map, tra cứu, tìm dạng gốc, gợi ý |
| `evdict/src/stardict.rs` | Chuyển StarDict sang `.dat` (dùng chung cho import và `builddict`) |
| `evdict/src/library.rs` | Danh sách từ điển đã cài, import từ file nén, cấu hình |
| `evdict/src/manager.rs` | Cửa sổ Quản lý từ điển |
| `evdict/src/render.rs` | Chuyển nội dung (chữ thường, HTML/XDXF/Pango) sang RTF theo theme |
| `evdict/src/popup.rs` | Cửa sổ hiển thị nghĩa (RichEdit), tự co giãn theo nội dung |
| `evdict/src/theme.rs` | Bảng màu sáng/tối theo hệ thống, font Windows 11/10 |
| `evdict/src/draw.rs` | Vẽ góc bo khử răng cưa và ô nhập kiểu Fluent (không cần GDI+) |
| `evdict/src/settings.rs` | Registry, khởi động cùng Windows, hộp thoại đổi phím tắt |
| `tools/builddict` | Chuyển StarDict sang `evdict.dat`: dữ liệu nén theo khối 64 KB, khoá được sắp xếp để tìm kiếm nhị phân |

## Dữ liệu

Từ điển Anh–Việt lấy từ [Open Vietnamese Dictionary Project (OVDP)](https://sourceforge.net/projects/ovdp/), giấy phép GPLv2.
