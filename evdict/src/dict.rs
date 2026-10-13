//! Đọc một từ điển evdict (.dat) bằng memory-map: chỉ những trang được truy cập mới nằm
//! trong RAM. Định dạng: xem stardict.rs (v2); v1 là bản cũ của dữ liệu có sẵn.

use std::cmp::Ordering;
use std::path::Path;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, GetFileSizeEx, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, OPEN_EXISTING,
};
use windows_sys::Win32::System::Memory::{
    CreateFileMappingW, MapViewOfFile, UnmapViewOfFile, FILE_MAP_READ, MEMORY_MAPPED_VIEW_ADDRESS,
    PAGE_READONLY,
};

use crate::key::norm_key;
use crate::util::wide;

pub struct Dict {
    data: &'static [u8],
    mapped: bool,
    pub name: String,
    n: usize,
    bs: usize,
    ent: usize,
    /// Kích thước một bản ghi mục: 8 (v1) hoặc 12 (v2).
    rec: usize,
    keys: usize,
    blk: usize,
}

impl Drop for Dict {
    fn drop(&mut self) {
        if self.mapped {
            unsafe {
                UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS { Value: self.data.as_ptr() as *mut _ });
            }
        }
    }
}

impl Dict {
    pub fn open(path: &Path) -> Option<Dict> {
        unsafe {
            let path = wide(path.to_str()?);
            let file = CreateFileW(
                path.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ,
                null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                null_mut(),
            );
            if file == INVALID_HANDLE_VALUE {
                return None;
            }
            let mut size = 0i64;
            GetFileSizeEx(file, &mut size);
            let mapping = CreateFileMappingW(file, null(), PAGE_READONLY, 0, 0, null());
            CloseHandle(file);
            if mapping.is_null() || size <= 0 {
                return None;
            }
            let view = MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, 0);
            CloseHandle(mapping);
            if view.Value.is_null() {
                return None;
            }
            let data = std::slice::from_raw_parts(view.Value as *const u8, size as usize);
            let d = Self::parse(data, true);
            if d.is_none() {
                UnmapViewOfFile(view);
            }
            d
        }
    }

    /// Đọc từ bộ nhớ (dùng cho kiểm thử).
    #[cfg(test)]
    pub fn from_bytes(bytes: Vec<u8>) -> Option<Dict> {
        Self::parse(Box::leak(bytes.into_boxed_slice()), false)
    }

    fn parse(data: &'static [u8], mapped: bool) -> Option<Dict> {
        if data.len() < 32 {
            return None;
        }
        let r = |o: usize| u32::from_le_bytes(data[o..o + 4].try_into().unwrap()) as usize;
        let (rec, name) = match &data[..4] {
            b"EVD1" => (8, "Anh - Việt".to_string()),
            b"EVD2" if data.len() >= 36 && 36 + r(32) <= data.len() => {
                (12, String::from_utf8_lossy(&data[36..36 + r(32)]).into_owned())
            }
            _ => return None,
        };
        let d = Dict { data, mapped, name, n: r(4), bs: r(8), ent: r(16), rec, keys: r(20), blk: r(24) };
        let nb = r(12);
        let ok = d.bs > 0
            && d.ent + (d.n + 1) * rec <= d.keys
            && d.keys <= d.blk
            && d.blk + (nb + 1) * 4 <= data.len()
            && d.u32(d.blk + nb * 4) == data.len();
        if ok {
            Some(d)
        } else {
            std::mem::forget(d); // chưa hợp lệ: không unmap hai lần
            None
        }
    }

    fn u32(&self, o: usize) -> usize {
        u32::from_le_bytes(self.data[o..o + 4].try_into().unwrap()) as usize
    }

    pub fn len(&self) -> usize {
        self.n
    }

    pub fn key(&self, i: usize) -> &[u8] {
        let a = self.u32(self.ent + i * self.rec);
        let b = self.u32(self.ent + (i + 1) * self.rec);
        &self.data[self.keys + a..self.keys + b]
    }

    pub fn find(&self, k: &[u8]) -> Result<usize, usize> {
        let (mut lo, mut hi) = (0, self.n);
        while lo < hi {
            let mid = (lo + hi) / 2;
            match self.key(mid).cmp(k) {
                Ordering::Less => lo = mid + 1,
                Ordering::Greater => hi = mid,
                Ordering::Equal => return Ok(mid),
            }
        }
        Err(lo)
    }

    /// Vùng dữ liệu (chưa nén) của mục i.
    fn range(&self, i: usize) -> (usize, usize) {
        let r = self.ent + i * self.rec;
        if self.rec == 12 {
            let d0 = self.u32(r + 4);
            (d0, d0 + self.u32(r + 8))
        } else {
            (self.u32(r + 4), self.u32(r + 8 + 4))
        }
    }

    pub fn text(&self, i: usize) -> String {
        let (d0, d1) = self.range(i);
        if d1 <= d0 {
            return String::new();
        }
        let mut out = Vec::with_capacity(d1 - d0);
        for b in d0 / self.bs..=(d1 - 1) / self.bs {
            let s = self.u32(self.blk + b * 4);
            let e = self.u32(self.blk + (b + 1) * 4);
            let block = miniz_oxide::inflate::decompress_to_vec(&self.data[s..e]).unwrap_or_default();
            let base = b * self.bs;
            let lo = d0.max(base) - base;
            let hi = d1.min(base + block.len()).saturating_sub(base);
            if lo < hi {
                out.extend_from_slice(&block[lo..hi]);
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }
}
pub enum Lookup {
    Found { idx: usize, note: Option<String> },
    NotFound { query: String, suggest: Vec<String> },
}

/// Làm sạch đoạn chữ được chọn thành chuỗi tra cứu.
pub fn clean(raw: &str) -> String {
    // Dấu câu (trừ ' - .) coi như khoảng trắng: "running,hello" -> "running hello".
    let raw: String = raw
        .chars()
        .take(200)
        .map(|c| if c.is_alphanumeric() || matches!(c, '\'' | '’' | '-' | '.') { c } else { ' ' })
        .collect();
    let words: Vec<&str> = raw
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|w| !w.is_empty())
        .take(6)
        .collect();
    norm_key(&words.join(" "))
}

pub fn lookup(d: &Dict, raw: &str) -> Lookup {
    let q = clean(raw);
    if q.is_empty() {
        return Lookup::NotFound { query: q, suggest: Vec::new() };
    }
    let words: Vec<&str> = q.split(' ').collect();
    // Thử cả cụm trước, rồi bớt dần từ cuối: "run out of time" -> "run out" -> "run".
    for k in (1..=words.len()).rev() {
        let cand = words[..k].join(" ");
        if let Ok(idx) = d.find(cand.as_bytes()) {
            let note = (k < words.len()).then(|| format!("Không có “{q}”, hiển thị “{cand}”"));
            return Lookup::Found { idx, note };
        }
        if k == 1 {
            for base in lemmas(&cand) {
                if let Ok(idx) = d.find(base.as_bytes()) {
                    let note = Some(format!("“{cand}” → dạng gốc “{base}”"));
                    return Lookup::Found { idx, note };
                }
            }
        }
    }
    Lookup::NotFound { suggest: suggestions(d, &q), query: q }
}

/// Gợi ý các từ có chung tiền tố dài nhất với chuỗi tra.
fn suggestions(d: &Dict, q: &str) -> Vec<String> {
    let ends: Vec<usize> = q.char_indices().map(|(i, c)| i + c.len_utf8()).collect();
    for &end in ends.iter().rev() {
        let prefix = &q.as_bytes()[..end];
        let pos = d.find(prefix).unwrap_or_else(|p| p);
        let hits: Vec<String> = (pos..(pos + 10).min(d.n))
            .map(|i| d.key(i))
            .take_while(|k| k.starts_with(prefix))
            .map(|k| String::from_utf8_lossy(k).into_owned())
            .collect();
        if !hits.is_empty() {
            return hits;
        }
    }
    Vec::new()
}

/// Các dạng gốc có thể có của một từ tiếng Anh (running -> run, studies -> study...).
fn lemmas(w: &str) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    let undouble = |b: &str, v: &mut Vec<String>| {
        let c: Vec<char> = b.chars().collect();
        let n = c.len();
        if n >= 3 && c[n - 1] == c[n - 2] && !"aeiou".contains(c[n - 1]) {
            v.push(c[..n - 1].iter().collect());
        }
    };
    if let Some(b) = w.strip_suffix("'s").or_else(|| w.strip_suffix("s'")) {
        v.push(b.into());
    }
    if let Some(b) = w.strip_suffix("ies") {
        v.push(format!("{b}y"));
    }
    if let Some(b) = w.strip_suffix("ves") {
        v.push(format!("{b}f"));
        v.push(format!("{b}fe"));
    }
    if let Some(b) = w.strip_suffix("es") {
        v.push(b.into());
    }
    if let Some(b) = w.strip_suffix('s') {
        if !w.ends_with("ss") {
            v.push(b.into());
        }
    }
    if let Some(b) = w.strip_suffix("ied") {
        v.push(format!("{b}y"));
    }
    if let Some(b) = w.strip_suffix("ed") {
        v.push(b.into());
        v.push(format!("{b}e"));
        undouble(b, &mut v);
    }
    if let Some(b) = w.strip_suffix("ying") {
        v.push(format!("{b}ie"));
    }
    if let Some(b) = w.strip_suffix("ing") {
        v.push(b.into());
        v.push(format!("{b}e"));
        undouble(b, &mut v);
    }
    for (suf, rep) in [("iest", "y"), ("ier", "y"), ("ily", "y"), ("ly", "le")] {
        if let Some(b) = w.strip_suffix(suf) {
            v.push(format!("{b}{rep}"));
        }
    }
    for suf in ["est", "er", "ly"] {
        if let Some(b) = w.strip_suffix(suf) {
            v.push(b.into());
            v.push(format!("{b}e"));
            undouble(b, &mut v);
        }
    }
    v.retain(|s| s.chars().count() >= 2 && s != w);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load() -> Dict {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../dist/evdict.dat");
        Dict::from_bytes(std::fs::read(path).expect("build dist first")).expect("valid file")
    }

    fn found(d: &Dict, q: &str) -> (String, Option<String>) {
        match lookup(d, q) {
            Lookup::Found { idx, note } => (String::from_utf8_lossy(d.key(idx)).into_owned(), note),
            Lookup::NotFound { .. } => panic!("not found: {q}"),
        }
    }

    #[test]
    fn exact_and_text() {
        let d = load();
        let (k, note) = found(&d, "Hello");
        assert_eq!(k, "hello");
        assert!(note.is_none());
        let i = d.find(b"run").unwrap();
        let t = d.text(i);
        assert!(t.starts_with("@run"), "{}", &t[..80]);
        assert!(t.contains("chạy"));
    }

    #[test]
    fn selection_cleanup_and_phrases() {
        let d = load();
        assert_eq!(found(&d, "  “dictionary,”\r\n").0, "dictionary");
        assert_eq!(found(&d, "run out").0, "run out");
        assert_eq!(found(&d, "give up smoking").0, "give up");
    }

    #[test]
    fn lemmas_find_base_word() {
        let d = load();
        for (q, want) in [("studies", "study"), ("stopped", "stop"), ("happier", "happy"), ("wolves", "wolf")] {
            let (k, _) = found(&d, q);
            assert!(k == q || k == want, "{q} -> {k}");
        }
    }

    #[test]
    fn not_found_gives_suggestions() {
        let d = load();
        match lookup(&d, "dictionaryzz") {
            Lookup::NotFound { suggest, .. } => {
                assert!(!suggest.is_empty());
                assert!(suggest.iter().all(|s| s.starts_with("dictionar")), "{suggest:?}");
            }
            Lookup::Found { .. } => panic!("unexpected"),
        }
    }
}
