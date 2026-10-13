//! Chuyển nội dung mục từ sang RTF cho RichEdit, theo màu của theme.
//! Hai kiểu nội dung: chữ thường kiểu OVDP (@ * - = !) và HTML/XDXF/Pango (xem stardict.rs).

use std::fmt::Write as _;

use crate::stardict::{MARK_HTML, MARK_PANGO, MARK_XDXF};
use crate::theme::{self, Theme};

// Font: 0 chữ thường, 1 tiêu đề.
// Màu: 1 chữ chính, 2 chữ phụ, 3 màu nhấn, 4 xanh lá (chuyên ngành, ghi chú), 5 ví dụ.
const CF_TEXT: u8 = 1;
const CF_TEXT2: u8 = 2;
const CF_ACCENT: u8 = 3;
const CF_GREEN: u8 = 4;
const CF_EXAMPLE: u8 = 5;

fn head(t: &Theme) -> String {
    let f = theme::fonts();
    let mut o = String::from(r"{\rtf1\ansi\deff0{\fonttbl");
    let _ = write!(o, r"{{\f0\fswiss {};}}{{\f1\fswiss {};}}}}{{\colortbl;", f.text, f.display);
    for c in [t.text, t.text2, t.accent, t.green, t.example] {
        let _ = write!(o, r"\red{}\green{}\blue{};", c >> 16, (c >> 8) & 0xFF, c & 0xFF);
    }
    o.push_str(r"}\f0\fs21\cf1 ");
    o
}

fn esc(o: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '\\' | '{' | '}' => {
                o.push('\\');
                o.push(c);
            }
            '\t' => o.push_str(r"\tab "),
            ' '..='~' => o.push(c),
            _ => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    let _ = write!(o, "\\u{}?", *u as i16);
                }
            }
        }
    }
}

fn para(o: &mut String, fmt: &str, text: &str, end: &str) {
    o.push_str(r"\pard");
    o.push_str(fmt);
    o.push(' ');
    esc(o, text);
    o.push_str(end);
    o.push_str(r"\par ");
}

/// Dòng tiêu đề: từ chính to, phiên âm nhạt.
fn headword(o: &mut String, word: &str, pron: &str, first: bool) {
    o.push_str(if first { r"\pard\sa60" } else { r"\pard\sb240\sa60" });
    o.push_str(r"\f1\fs36\b\cf1 ");
    esc(o, word);
    o.push_str(r"\b0\f0\fs22\cf2   ");
    esc(o, pron);
    o.push_str(r"\par ");
}

pub fn entry(t: &Theme, text: &str, note: Option<&str>) -> String {
    let mut o = head(t);
    if let Some(n) = note {
        para(&mut o, r"\sa100\fs18\cf2\i", n, r"\i0");
    }
    match text.as_bytes().first() {
        Some(&k) if matches!(k, MARK_HTML | MARK_XDXF | MARK_PANGO) => {
            let (word, body) = text[1..].split_once('\n').unwrap_or((&text[1..], ""));
            headword(&mut o, word.trim(), "", true);
            Markup::new(&mut o, k).run(body);
        }
        _ => plain(&mut o, text),
    }
    o.push('}');
    o
}

fn plain(o: &mut String, text: &str) {
    let mut first_head = true;
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        if let Some(rest) = l.strip_prefix('@') {
            if rest.starts_with("Chuyên ngành") {
                para(o, r"\sb240\sa60\fs19\b\cf4", &rest.to_uppercase(), r"\b0");
            } else if rest.starts_with("Lĩnh vực") {
                para(o, r"\sb80\sa20\fs18\i\cf2", rest, r"\i0");
            } else {
                // Từ chính + phiên âm: "run /rʌn/"
                let (word, pron) = match rest.find('/') {
                    Some(i) => (rest[..i].trim(), &rest[i..]),
                    None => (rest.trim(), ""),
                };
                headword(o, word, pron, first_head);
                first_head = false;
            }
        } else if let Some(rest) = l.strip_prefix('*') {
            para(o, r"\sb180\sa60\fs20\b\cf3", rest.trim(), r"\b0");
        } else if let Some(rest) = l.strip_prefix('-') {
            para(o, r"\li280\fi-200\sb40\sa20\fs21\cf1", &format!("•  {}", rest.trim()), "");
        } else if let Some(rest) = l.strip_prefix('=') {
            let (ex, tr) = rest.split_once('+').unwrap_or((rest, ""));
            o.push_str(r"\pard\li520\sa20\fs19\i\cf5 ");
            esc(o, ex.trim());
            o.push_str(r"\i0\cf2 ");
            if !tr.trim().is_empty() {
                esc(o, &format!("  —  {}", tr.trim()));
            }
            o.push_str(r"\par ");
        } else if let Some(rest) = l.strip_prefix('!') {
            para(o, r"\sb120\li280\sa20\fs21\b\cf1", rest.trim(), r"\b0");
        } else {
            para(o, r"\li280\sa20\fs21\cf1", l, "");
        }
    }
}

pub fn message(t: &Theme, msg: &str, suggest: &[String]) -> String {
    let mut o = head(t);
    para(&mut o, r"\sb40\sa140\fs21\cf2", msg, "");
    if !suggest.is_empty() {
        para(&mut o, r"\sa60\fs18\b\cf2", "TỪ GẦN GIỐNG", r"\b0");
        for s in suggest {
            para(&mut o, r"\sa30\fs21\cf3", s, "");
        }
    }
    o.push('}');
    o
}

// ---------- HTML / XDXF / Pango ----------

#[derive(Clone, Copy, PartialEq)]
struct Style {
    bold: bool,
    italic: bool,
    underline: bool,
    color: u8,
    size: u8, // nửa point
    script: i8, // 1 chỉ số trên, -1 chỉ số dưới
    indent: u16,
}

const BASE: Style = Style { bold: false, italic: false, underline: false, color: CF_TEXT, size: 21, script: 0, indent: 0 };

struct Markup<'a> {
    o: &'a mut String,
    kind: u8,
    stack: Vec<(String, Style)>,
    /// Đoạn hiện tại đã có chữ chưa (để không tạo đoạn trống).
    has_text: bool,
    /// Đã mở đoạn (\pard) chưa.
    open: bool,
    pending_space: bool,
    bullet: bool,
}

impl<'a> Markup<'a> {
    fn new(o: &'a mut String, kind: u8) -> Self {
        Markup { o, kind, stack: Vec::new(), has_text: false, open: false, pending_space: false, bullet: false }
    }

    fn style(&self) -> Style {
        self.stack.last().map(|s| s.1).unwrap_or(BASE)
    }

    fn break_para(&mut self) {
        if self.has_text {
            self.o.push_str(r"\par ");
        }
        self.has_text = false;
        self.open = false;
        self.pending_space = false;
    }

    /// Ghi một đoạn chữ cùng kiểu định dạng thành một nhóm RTF.
    fn emit(&mut self, run: &str) {
        if run.is_empty() {
            return;
        }
        let st = self.style();
        if !self.open {
            let _ = write!(self.o, r"\pard\sa40\li{} ", 280 + st.indent as u32 * 280);
            self.open = true;
            if std::mem::take(&mut self.bullet) {
                self.o.push_str(r"舦?  ");
            }
        }
        self.o.push('{');
        if st.bold {
            self.o.push_str(r"\b");
        }
        if st.italic {
            self.o.push_str(r"\i");
        }
        if st.underline {
            self.o.push_str(r"\ul");
        }
        match st.script {
            1 => self.o.push_str(r"\super"),
            -1 => self.o.push_str(r"\sub"),
            _ => {}
        }
        let _ = write!(self.o, r"\cf{}\fs{} ", st.color, st.size);
        esc(self.o, run);
        self.o.push('}');
        self.has_text = true;
    }

    /// Chữ giữa các thẻ. HTML gộp khoảng trắng; XDXF/Pango coi xuống dòng là ngắt đoạn.
    fn text(&mut self, s: &str) {
        let collapse = self.kind == MARK_HTML;
        for (i, line) in s.split('\n').enumerate() {
            if i > 0 && !collapse {
                self.break_para();
            }
            let mut run = String::new();
            for c in line.chars() {
                if c.is_whitespace() {
                    if self.has_text || !run.is_empty() {
                        self.pending_space = true;
                    }
                    continue;
                }
                if std::mem::take(&mut self.pending_space) {
                    run.push(' ');
                }
                run.push(c);
            }
            self.emit(&run);
        }
    }

    fn open_tag(&mut self, name: &str, attrs: &str) {
        let mut st = self.style();
        let xdxf = self.kind == MARK_XDXF;
        let block = matches!(
            name,
            "p" | "div" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "blockquote" | "dd" | "dt"
                | "ul" | "ol" | "table" | "hr" | "def" | "dtrn"
        ) || (name == "tr" && !xdxf);
        if block {
            self.break_para();
        }
        match name {
            "b" | "strong" => st.bold = true,
            "i" | "em" | "cite" | "var" => st.italic = true,
            "u" | "ins" => st.underline = true,
            "big" => st.size = 25,
            "small" => st.size = 18,
            "sup" => st.script = 1,
            "sub" => st.script = -1,
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                st.bold = true;
                st.size = 24;
            }
            "a" | "kref" | "iref" => st.color = CF_ACCENT,
            "font" | "span" if attrs.contains("color") || attrs.contains("foreground") => st.color = CF_ACCENT,
            "blockquote" | "dd" | "ul" | "ol" => st.indent += 1,
            "li" => self.bullet = true,
            // XDXF
            "k" if xdxf => {
                st.bold = true;
                st.size = 26;
            }
            "tr" if xdxf => st.color = CF_TEXT2,
            "ex" => {
                st.italic = true;
                st.color = CF_EXAMPLE;
            }
            "abr" | "co" | "gr" => {
                st.italic = true;
                st.color = CF_GREEN;
            }
            "c" if xdxf => st.color = CF_ACCENT,
            _ => {}
        }
        self.stack.push((name.to_string(), st));
        if name == "tr" && xdxf {
            self.text("/");
        }
    }

    fn close_tag(&mut self, name: &str) {
        if name == "tr" && self.kind == MARK_XDXF {
            self.text("/");
        }
        if let Some(pos) = self.stack.iter().rposition(|(n, _)| n == name) {
            self.stack.truncate(pos);
        }
        let block = matches!(
            name,
            "p" | "div" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "blockquote" | "dd" | "dt"
                | "ul" | "ol" | "table" | "def" | "dtrn"
        ) || (name == "tr" && self.kind != MARK_XDXF);
        if block {
            self.break_para();
        }
    }

    fn run(mut self, src: &str) {
        let mut rest = src;
        while !rest.is_empty() {
            match rest.find('<') {
                Some(0) => {
                    if let Some(after) = rest.strip_prefix("<!--") {
                        rest = after.find("-->").map_or("", |e| &after[e + 3..]);
                        continue;
                    }
                    let Some(end) = rest.find('>') else {
                        self.text(&decode(rest));
                        break;
                    };
                    let tag = rest[1..end].trim();
                    rest = &rest[end + 1..];
                    let closing = tag.starts_with('/');
                    let tag = tag.trim_start_matches('/').trim_end_matches('/');
                    let (name, attrs) = tag.split_once(char::is_whitespace).unwrap_or((tag, ""));
                    let name = name.to_ascii_lowercase();
                    match name.as_str() {
                        "br" => {
                            if self.has_text {
                                self.o.push_str(r"\line ");
                                self.pending_space = false;
                            }
                        }
                        // Bỏ cả nội dung của script/style.
                        "script" | "style" if !closing => {
                            let close = format!("</{name}");
                            let lower = rest.to_ascii_lowercase();
                            rest = lower.find(&close).map_or("", |e| {
                                let after = &rest[e..];
                                after.find('>').map_or("", |g| &after[g + 1..])
                            });
                        }
                        "img" | "meta" | "link" | "input" => {}
                        "hr" => self.break_para(),
                        _ if closing => self.close_tag(&name),
                        _ => self.open_tag(&name, attrs),
                    }
                }
                Some(i) => {
                    let (t, r) = rest.split_at(i);
                    self.text(&decode(t));
                    rest = r;
                }
                None => {
                    self.text(&decode(rest));
                    break;
                }
            }
        }
        if self.has_text {
            self.o.push_str(r"\par ");
        }
    }
}

/// Giải mã các thực thể HTML thường gặp.
fn decode(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest[..rest.len().min(12)].find(';');
        let decoded = end.and_then(|e| {
            let ent = &rest[1..e];
            let c = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                    u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32)
                }
                _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
                _ => None,
            };
            c.map(|c| (c, e))
        });
        match decoded {
            Some((c, e)) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rtf(kind: u8, body: &str) -> String {
        let mut o = String::new();
        Markup::new(&mut o, kind).run(body);
        o
    }

    #[test]
    fn html_basic() {
        let o = rtf(MARK_HTML, "<b>run</b> &amp; <i>chạy</i><br>dòng 2<script>x()</script><p>đoạn</p>");
        assert!(o.contains(r"{\b\cf1\fs21 run}"), "{o}");
        assert!(o.contains(r"{\cf1\fs21  &}"), "{o}");
        assert!(o.contains(r"\line "), "{o}");
        assert!(!o.contains("x()"), "{o}");
        assert!(o.matches(r"\par ").count() >= 2, "{o}");
    }

    #[test]
    fn xdxf_transcription_and_newlines() {
        let o = rtf(MARK_XDXF, "<k>cat</k>\n<tr>kæt</tr>\n<ex>a cat</ex>");
        assert!(o.matches(r"\par ").count() >= 3, "{o}");
        assert!(o.contains(r"{\cf2\fs21 /}"), "{o}");
    }

    #[test]
    fn entities() {
        assert_eq!(decode("a&lt;b&gt;&#65;&#x42;&foo;"), "a<b>AB&foo;");
    }
}
