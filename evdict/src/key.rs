/// Chuẩn hoá khoá tra cứu: gộp khoảng trắng, chữ thường, thống nhất dấu nháy.
/// Dùng chung cho công cụ tạo dữ liệu và ứng dụng để thứ tự sắp xếp khớp nhau.
pub fn norm_key(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for word in s.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        for c in word.chars() {
            let c = match c {
                '\u{2018}' | '\u{2019}' => '\'',
                _ => c,
            };
            out.extend(c.to_lowercase());
        }
    }
    out
}
