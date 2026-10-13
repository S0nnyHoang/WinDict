//! Danh sách từ điển đã cài: từ điển có sẵn (evdict.dat cạnh exe) và các bộ người dùng
//! import (trong %APPDATA%\EVDict\dicts). Thứ tự = độ ưu tiên khi hiển thị tab.
//! Cấu hình lưu ở %APPDATA%\EVDict\dicts.txt, mỗi dòng: bật(0/1) \t file \t tên hiển thị.

use std::cell::{OnceCell, RefCell};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::dict::Dict;
use crate::stardict::{self, Source};
use crate::util::exe_dir;

pub const BUILTIN: &str = "builtin";

pub struct Installed {
    pub file: String,
    pub enabled: bool,
    /// Tên do người dùng đặt; trống thì dùng tên trong file.
    pub title: String,
    dict: OnceCell<Option<Rc<Dict>>>,
}

impl Installed {
    fn new(file: String, enabled: bool, title: String) -> Self {
        Installed { file, enabled, title, dict: OnceCell::new() }
    }

    pub fn path(&self) -> PathBuf {
        if self.file == BUILTIN {
            exe_dir().join("evdict.dat")
        } else {
            data_dir().join(&self.file)
        }
    }

    /// Mở (memory-map) ở lần dùng đầu tiên.
    pub fn dict(&self) -> Option<Rc<Dict>> {
        self.dict.get_or_init(|| Dict::open(&self.path()).map(Rc::new)).clone()
    }

    pub fn display_name(&self) -> String {
        if !self.title.is_empty() {
            return self.title.clone();
        }
        match self.dict() {
            Some(d) if !d.name.is_empty() => pretty(&d.name),
            _ => self.file.clone(),
        }
    }
}

/// "star_vietanh" -> "star vietanh"
fn pretty(name: &str) -> String {
    name.replace('_', " ")
}

thread_local! {
    static LIB: RefCell<Option<Vec<Installed>>> = const { RefCell::new(None) };
}

pub fn app_dir() -> PathBuf {
    // EVDICT_HOME: thư mục dữ liệu riêng (kiểm thử, hoặc chạy di động từ USB).
    if let Some(home) = std::env::var_os("EVDICT_HOME") {
        return PathBuf::from(home);
    }
    // Bản portable: có file portable.txt cạnh exe thì lưu dữ liệu ngay trong thư mục đó.
    if exe_dir().join("portable.txt").exists() {
        return exe_dir().join("data");
    }
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(exe_dir);
    base.join("EVDict")
}

pub fn data_dir() -> PathBuf {
    app_dir().join("dicts")
}

fn config_path() -> PathBuf {
    app_dir().join("dicts.txt")
}

fn load() -> Vec<Installed> {
    let mut list: Vec<Installed> = std::fs::read_to_string(config_path())
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let mut f = line.split('\t');
            let enabled = f.next()? == "1";
            let file = f.next()?.to_string();
            let title = f.next().unwrap_or("").to_string();
            (!file.is_empty()).then(|| Installed::new(file, enabled, title))
        })
        .collect();
    // Bỏ các file đã bị xoá bằng tay; thêm các file .dat mới chép vào thư mục.
    list.retain(|i| i.file == BUILTIN || data_dir().join(&i.file).exists());
    if !list.iter().any(|i| i.file == BUILTIN) {
        list.insert(0, Installed::new(BUILTIN.into(), true, String::new()));
    }
    if let Ok(rd) = std::fs::read_dir(data_dir()) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.ends_with(".dat") && !list.iter().any(|i| i.file == name) {
                list.push(Installed::new(name, true, String::new()));
            }
        }
    }
    list
}

fn save(list: &[Installed]) {
    let text: String = list
        .iter()
        .map(|i| format!("{}\t{}\t{}\n", i.enabled as u8, i.file, i.title.replace(['\t', '\n'], " ")))
        .collect();
    let _ = std::fs::create_dir_all(app_dir());
    let _ = std::fs::write(config_path(), text);
}

/// Làm việc với danh sách; `changed` = true thì ghi lại cấu hình.
pub fn with<R>(f: impl FnOnce(&mut Vec<Installed>) -> (R, bool)) -> R {
    LIB.with(|lib| {
        let mut lib = lib.borrow_mut();
        let list = lib.get_or_insert_with(load);
        let (r, changed) = f(list);
        if changed {
            save(list);
        }
        r
    })
}

/// Các từ điển đang bật, theo thứ tự ưu tiên: (tên hiển thị, từ điển).
pub fn enabled() -> Vec<(String, Rc<Dict>)> {
    with(|list| {
        let v = list
            .iter()
            .filter(|i| i.enabled)
            .filter_map(|i| i.dict().map(|d| (i.display_name(), d)))
            .collect();
        (v, false)
    })
}

/// Thêm các file vừa import vào cuối danh sách (đang bật).
pub fn add(files: &[String]) {
    with(|list| {
        for f in files {
            if !list.iter().any(|i| &i.file == f) {
                list.push(Installed::new(f.clone(), true, String::new()));
            }
        }
        ((), true)
    })
}

/// Gỡ một từ điển đã import (không gỡ được bản có sẵn) và xoá file.
pub fn remove(index: usize) -> Result<(), String> {
    let path = with(|list| {
        if index >= list.len() || list[index].file == BUILTIN {
            return (None, false);
        }
        let item = list.remove(index);
        (Some(item.path()), true) // `item` bị huỷ ở đây: unmap trước khi xoá file
    });
    match path {
        Some(p) => std::fs::remove_file(&p).map_err(|e| format!("không xoá được {}: {e}", p.display())),
        None => Err("Từ điển có sẵn không xoá được, bạn có thể bỏ chọn để tắt.".into()),
    }
}

// ---------- Import ----------

#[derive(Debug)]
pub struct Imported {
    pub file: String,
    pub name: String,
    pub entries: usize,
}

/// Các phần của một bộ StarDict tìm thấy trong file/thư mục/file nén, theo tên gốc.
#[derive(Default)]
struct Parts {
    ifo: Option<Vec<u8>>,
    idx: Option<Vec<u8>>,
    dict: Option<Vec<u8>>,
    syn: Option<Vec<u8>>,
}

/// Tách "thư/mục/abc.dict.dz" thành ("thư/mục/abc", phần) nếu là file của StarDict.
fn classify(name: &str) -> Option<(String, &'static str)> {
    let lower = name.to_ascii_lowercase();
    for (suffix, part) in [
        (".ifo", "ifo"),
        (".idx.gz", "idx"),
        (".idx.dz", "idx"),
        (".idx", "idx"),
        (".dict.dz", "dict"),
        (".dict.gz", "dict"),
        (".dict", "dict"),
        (".syn.dz", "syn"),
        (".syn.gz", "syn"),
        (".syn", "syn"),
    ] {
        if lower.ends_with(suffix) {
            return Some((name[..name.len() - suffix.len()].to_string(), part));
        }
    }
    None
}

fn collect(found: &mut BTreeMap<String, Parts>, name: &str, data: impl FnOnce() -> std::io::Result<Vec<u8>>) -> Result<(), String> {
    if let Some((base, part)) = classify(name) {
        let bytes = data().map_err(|e| format!("đọc {name} lỗi: {e}"))?;
        let p = found.entry(base).or_default();
        match part {
            "ifo" => p.ifo = Some(bytes),
            "idx" => p.idx = Some(bytes),
            "dict" => p.dict = Some(bytes),
            _ => p.syn = Some(bytes),
        }
    }
    Ok(())
}

fn read_tar<R: Read>(reader: R, found: &mut BTreeMap<String, Parts>) -> Result<(), String> {
    let mut ar = tar::Archive::new(reader);
    for entry in ar.entries().map_err(|e| format!("file tar lỗi: {e}"))? {
        let mut entry = entry.map_err(|e| format!("file tar lỗi: {e}"))?;
        let name = entry.path().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        collect(found, &name, || {
            let mut v = Vec::new();
            entry.read_to_end(&mut v).map(|_| v)
        })?;
    }
    Ok(())
}

/// Đọc các bộ StarDict từ: file .ifo (lấy các file cùng tên bên cạnh), .zip, .tar(.gz/.bz2).
fn read_sources(path: &Path) -> Result<Vec<(String, Source)>, String> {
    let lower = path.to_string_lossy().to_ascii_lowercase();
    let mut found: BTreeMap<String, Parts> = BTreeMap::new();
    let open = || std::fs::File::open(path).map_err(|e| format!("không mở được file: {e}"));

    if lower.ends_with(".ifo") {
        let base = &path.to_string_lossy()[..path.to_string_lossy().len() - 4];
        for ext in [".ifo", ".idx", ".idx.gz", ".dict.dz", ".dict", ".syn", ".syn.dz"] {
            let f = format!("{base}{ext}");
            if Path::new(&f).exists() {
                collect(&mut found, &f, || std::fs::read(&f))?;
            }
        }
    } else if lower.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(open()?).map_err(|e| format!("file zip lỗi: {e}"))?;
        for i in 0..zip.len() {
            let mut f = zip.by_index(i).map_err(|e| format!("file zip lỗi: {e}"))?;
            let name = f.name().map(|n| n.into_owned()).unwrap_or_default();
            collect(&mut found, &name, || {
                let mut v = Vec::new();
                f.read_to_end(&mut v).map(|_| v)
            })?;
        }
    } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        read_tar(flate2::read::GzDecoder::new(open()?), &mut found)?;
    } else if lower.ends_with(".tar.bz2") || lower.ends_with(".tbz2") || lower.ends_with(".tbz") {
        read_tar(bzip2::read::BzDecoder::new(open()?), &mut found)?;
    } else if lower.ends_with(".tar") {
        read_tar(open()?, &mut found)?;
    } else {
        return Err("Chỉ hỗ trợ file .ifo, .zip, .tar, .tar.gz, .tar.bz2 của từ điển StarDict.".into());
    }

    let mut out = Vec::new();
    for (base, p) in found {
        if let (Some(ifo), Some(idx), Some(dict)) = (p.ifo, p.idx, p.dict) {
            let src = Source {
                ifo,
                idx: stardict::maybe_gunzip(idx)?,
                dict: stardict::maybe_gunzip(dict)?,
                syn: p.syn.map(stardict::maybe_gunzip).transpose()?,
            };
            out.push((base, src));
        }
    }
    if out.is_empty() {
        return Err("Không tìm thấy từ điển StarDict (.ifo + .idx + .dict) trong file này.".into());
    }
    Ok(out)
}

/// Tên file an toàn, không trùng trong thư mục từ điển.
fn unique_file(name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug = if slug.is_empty() { "dict".to_string() } else { slug.chars().take(40).collect() };
    let mut n = 1;
    loop {
        let f = if n == 1 { format!("{slug}.dat") } else { format!("{slug}-{n}.dat") };
        if !data_dir().join(&f).exists() {
            return f;
        }
        n += 1;
    }
}

/// Import một file (có thể chứa nhiều bộ). Chạy được ở luồng phụ: chỉ ghi file,
/// không đụng vào danh sách; gọi `add` ở luồng giao diện sau đó.
pub fn import_file(path: &Path, progress: &mut dyn FnMut(u32)) -> Result<Vec<Imported>, String> {
    let sources = read_sources(path)?;
    std::fs::create_dir_all(data_dir()).map_err(|e| format!("không tạo được thư mục: {e}"))?;
    let count = sources.len() as u32;
    let mut out = Vec::new();
    for (i, (_, src)) in sources.iter().enumerate() {
        let mut sub = |p: u32| progress((i as u32 * 100 + p) / count);
        let conv = stardict::convert(src, None, 6, &mut sub)?;
        let file = unique_file(&conv.name);
        let dest = data_dir().join(&file);
        let tmp = dest.with_extension("tmp");
        std::fs::write(&tmp, &conv.bytes).map_err(|e| format!("ghi file lỗi: {e}"))?;
        std::fs::rename(&tmp, &dest).map_err(|e| format!("ghi file lỗi: {e}"))?;
        out.push(Imported { file, name: pretty(&conv.name), entries: conv.entries });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dict::{lookup, Lookup};
    use std::io::Write;
    use std::sync::OnceLock;

    /// Thư mục dữ liệu tạm cho kiểm thử (không đụng vào %APPDATA%).
    fn home() -> PathBuf {
        static H: OnceLock<PathBuf> = OnceLock::new();
        H.get_or_init(|| {
            let d = std::env::temp_dir().join(format!("evdict-test-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            std::env::set_var("EVDICT_HOME", &d);
            d
        })
        .clone()
    }

    /// Tạo (ifo, idx, dict) StarDict từ các mục (từ, dữ liệu thô).
    fn make(name: &str, entries: &[(&str, Vec<u8>)], types: Option<&str>, bits64: bool) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let mut idx = Vec::new();
        let mut dict = Vec::new();
        for (w, data) in entries {
            idx.extend_from_slice(w.as_bytes());
            idx.push(0);
            if bits64 {
                idx.extend_from_slice(&(dict.len() as u64).to_be_bytes());
            } else {
                idx.extend_from_slice(&(dict.len() as u32).to_be_bytes());
            }
            idx.extend_from_slice(&(data.len() as u32).to_be_bytes());
            dict.extend_from_slice(data);
        }
        let mut ifo = format!("StarDict's dict ifo file\nversion=3.0.0\nbookname={name}\nwordcount={}\n", entries.len());
        if let Some(t) = types {
            ifo.push_str(&format!("sametypesequence={t}\n"));
        }
        if bits64 {
            ifo.push_str("idxoffsetbits=64\n");
        }
        (ifo.into_bytes(), idx, dict)
    }

    fn syn(entries: &[(&str, u32)]) -> Vec<u8> {
        let mut v = Vec::new();
        for (w, i) in entries {
            v.extend_from_slice(w.as_bytes());
            v.push(0);
            v.extend_from_slice(&i.to_be_bytes());
        }
        v
    }

    fn open(file: &str) -> Dict {
        Dict::open(&data_dir().join(file)).expect("open imported dict")
    }

    fn text_of(d: &Dict, q: &str) -> String {
        match lookup(d, q) {
            Lookup::Found { idx, .. } => d.text(idx),
            Lookup::NotFound { .. } => panic!("not found: {q}"),
        }
    }

    #[test]
    fn import_zip_with_two_dicts_plain_html_and_synonyms() {
        let dir = home();
        let (ifo_a, idx_a, dict_a) = make(
            "plain_test",
            &[("Apple", b"@apple /'aepl/\n* noun\n- qua tao".to_vec()), ("zebra", b"ngua van".to_vec())],
            Some("m"),
            false,
        );
        // Tự mô tả (không có sametypesequence): phần 't' (phiên âm) rồi 'h' (HTML).
        let mut e1 = b"thelou\0h".to_vec();
        e1.extend_from_slice(b"<b>hello</b> &amp; <i>xin ch\xc3\xa0o</i>\0");
        let (ifo_b, idx_b, dict_b) = make("html_test", &[("hello", e1)], None, false);

        let zip_path = dir.join("two.zip");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        let opt = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name, data) in [
            ("a/plain.ifo", &ifo_a),
            ("a/plain.idx", &idx_a),
            ("a/plain.dict", &dict_a),
            ("b/html.ifo", &ifo_b),
            ("b/html.idx", &idx_b),
            ("b/html.dict", &dict_b),
        ] {
            z.start_file(name, opt).unwrap();
            z.write_all(data).unwrap();
        }
        z.start_file("b/html.syn", opt).unwrap();
        z.write_all(&syn(&[("hi", 0), ("HELLO", 0)])).unwrap();
        z.finish().unwrap();

        let mut last = 0;
        let out = import_file(&zip_path, &mut |p| last = p).unwrap();
        assert_eq!(last, 100);
        assert_eq!(out.len(), 2);
        let plain = out.iter().find(|i| i.name == "plain test").unwrap();
        let html = out.iter().find(|i| i.name == "html test").unwrap();
        assert_eq!(plain.entries, 2);
        assert_eq!(html.entries, 2, "hello + synonym hi (HELLO trùng khoá nên bỏ)");

        let d = open(&plain.file);
        assert_eq!(d.name, "plain_test");
        assert!(text_of(&d, "apples").contains("qua tao"), "lemma apples -> apple");

        let d = open(&html.file);
        let t = text_of(&d, "hi");
        assert_eq!(t.as_bytes()[0], stardict::MARK_HTML);
        assert!(t.contains("/helou/") &&t.contains("<b>hello</b>") && t.contains("xin chào"), "{t}");
    }

    #[test]
    fn import_tar_bz2_with_64bit_offsets_and_dictzip() {
        let dir = home();
        let (ifo, idx, dict) = make("big_offsets", &[("cat", b"con meo".to_vec()), ("dog", b"con cho".to_vec())], Some("m"), true);
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&dict).unwrap();
        let dict_dz = gz.finish().unwrap();

        let path = dir.join("pack.tar.bz2");
        let enc = bzip2::write::BzEncoder::new(std::fs::File::create(&path).unwrap(), bzip2::Compression::default());
        let mut tar = tar::Builder::new(enc);
        for (name, data) in [("x/big.ifo", &ifo), ("x/big.idx", &idx), ("x/big.dict.dz", &dict_dz)] {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            tar.append_data(&mut h, name, &data[..]).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();

        let out = import_file(&path, &mut |_| {}).unwrap();
        assert_eq!(out.len(), 1);
        let d = open(&out[0].file);
        assert!(text_of(&d, "dog").contains("con cho"));
        assert!(text_of(&d, "Cats").contains("con meo"));
    }

    #[test]
    fn library_add_toggle_reorder_remove() {
        let dir = home();
        let (ifo, idx, dict) = make("lib_test", &[("sun", b"mat troi".to_vec())], Some("m"), false);
        std::fs::write(dir.join("lib.ifo"), ifo).unwrap();
        std::fs::write(dir.join("lib.idx"), idx).unwrap();
        std::fs::write(dir.join("lib.dict"), dict).unwrap();
        let out = import_file(&dir.join("lib.ifo"), &mut |_| {}).unwrap();
        let file = out[0].file.clone();
        add(std::slice::from_ref(&file));

        let pos = with(|l| (l.iter().position(|i| i.file == file), false)).unwrap();
        assert!(enabled().iter().any(|(n, _)| n == "lib test"));
        with(|l| {
            l[pos].enabled = false;
            l[pos].title = "Mặt trời".into();
            ((), true)
        });
        assert!(!enabled().iter().any(|(n, _)| n == "Mặt trời"));
        let saved = std::fs::read_to_string(config_path()).unwrap();
        assert!(saved.contains(&format!("0\t{file}\tMặt trời")), "{saved}");

        assert!(remove(0).is_err() || with(|l| (l[0].file != BUILTIN, false)), "không xoá được từ điển có sẵn");
        let pos = with(|l| (l.iter().position(|i| i.file == file), false)).unwrap();
        remove(pos).unwrap();
        assert!(!data_dir().join(&file).exists());
    }

    #[test]
    fn rejects_non_stardict() {
        let dir = home();
        let p = dir.join("x.zip");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
        z.start_file("readme.txt", zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(b"hi").unwrap();
        z.finish().unwrap();
        assert!(import_file(&p, &mut |_| {}).unwrap_err().contains("Không tìm thấy"));
        assert!(import_file(&dir.join("a.rar"), &mut |_| {}).is_err());
    }

    /// Bộ thật (OVDP VietAnh.zip): đặt EVDICT_SAMPLE_ZIP để chạy.
    #[test]
    fn import_real_sample_if_available() {
        let Some(p) = std::env::var_os("EVDICT_SAMPLE_ZIP") else { return };
        home();
        let t = std::time::Instant::now();
        let out = import_file(Path::new(&p), &mut |_| {}).unwrap();
        eprintln!("imported {:?} in {:?}", out.iter().map(|i| (&i.name, i.entries)).collect::<Vec<_>>(), t.elapsed());
        let va = out.iter().find(|i| i.name.contains("vietanh")).unwrap();
        let d = open(&va.file);
        let t = text_of(&d, "mèo");
        eprintln!("mèo => {}", t.lines().take(4).collect::<Vec<_>>().join(" | "));
        assert!(t.to_lowercase().contains("cat"));
    }
}
