//! Đọc từ điển StarDict (.ifo + .idx + .dict[.dz] + .syn) và ghi ra định dạng evdict.
//! Dùng chung cho app (import từ điển) và tools/builddict (tạo dữ liệu có sẵn).
//!
//! Định dạng evdict v2 (little endian):
//!   0  "EVD2"
//!   4  n        số mục
//!   8  bs       kích thước khối dữ liệu chưa nén
//!   12 nb       số khối
//!   16 ent      bảng mục: (n+1) bản ghi (key_off, data_off, data_len) u32, sắp theo khoá
//!   20 keys     vùng khoá (UTF-8 đã chuẩn hoá, nối liền; bản ghi cuối chỉ để lấy độ dài khoá cuối)
//!   24 blk      bảng khối: (nb+1) vị trí tuyệt đối của từng khối nén (raw deflate)
//!   28 total    tổng độ dài dữ liệu chưa nén
//!   32 name_len, 36.. tên từ điển (UTF-8)
//! Nhiều khoá có thể trỏ cùng một vùng dữ liệu (từ đồng nghĩa trong .syn).
//!
//! Nội dung một mục: chữ thường kiểu OVDP (dòng bắt đầu bằng @ * - = !), hoặc nếu byte
//! đầu là MARK_* thì: byte đánh dấu + từ chính + '\n' + nội dung HTML/XDXF/Pango.

use std::collections::{BTreeMap, HashMap};
use std::io::Read;

use crate::key::norm_key;

pub const BLOCK: usize = 64 * 1024;
pub const MARK_HTML: u8 = 1;
pub const MARK_XDXF: u8 = 2;
pub const MARK_PANGO: u8 = 3;

/// Các file của một từ điển StarDict, đã giải nén .gz/.dz.
pub struct Source {
    pub ifo: Vec<u8>,
    pub idx: Vec<u8>,
    pub dict: Vec<u8>,
    pub syn: Option<Vec<u8>>,
}

pub struct Converted {
    pub name: String,
    pub entries: usize,
    pub bytes: Vec<u8>,
}

/// Giải nén nếu dữ liệu là gzip/dictzip, không thì giữ nguyên.
pub fn maybe_gunzip(data: Vec<u8>) -> Result<Vec<u8>, String> {
    if data.starts_with(&[0x1F, 0x8B]) {
        let mut out = Vec::new();
        flate2::read::MultiGzDecoder::new(&data[..])
            .read_to_end(&mut out)
            .map_err(|e| format!("giải nén gzip lỗi: {e}"))?;
        Ok(out)
    } else {
        Ok(data)
    }
}

struct Info {
    name: String,
    offset_bits: u32,
    sametypes: Option<Vec<u8>>,
}

fn parse_ifo(ifo: &[u8]) -> Result<Info, String> {
    let text = String::from_utf8_lossy(ifo);
    let mut lines = text.lines();
    if !lines.next().unwrap_or("").contains("StarDict's dict ifo file") {
        return Err("file .ifo không đúng định dạng StarDict".into());
    }
    let mut info = Info { name: String::new(), offset_bits: 32, sametypes: None };
    for line in lines {
        if let Some((k, v)) = line.split_once('=') {
            match k.trim() {
                "bookname" => info.name = v.trim().to_string(),
                "idxoffsetbits" => info.offset_bits = v.trim().parse().unwrap_or(32),
                "sametypesequence" => info.sametypes = Some(v.trim().as_bytes().to_vec()),
                _ => {}
            }
        }
    }
    Ok(info)
}

fn esc_markup(s: &str, kind: u8) -> String {
    let s = s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    if kind == MARK_HTML {
        s.replace('\n', "<br>")
    } else {
        s
    }
}

/// Tách nội dung một mục thành các phần (loại, dữ liệu) theo sametypesequence hoặc tự mô tả.
fn parts<'a>(data: &'a [u8], sametypes: Option<&[u8]>) -> Vec<(u8, &'a [u8])> {
    let mut out = Vec::new();
    let mut p = 0;
    let take = |p: &mut usize, t: u8, last: bool| -> Option<&'a [u8]> {
        if last {
            let s = &data[*p..];
            *p = data.len();
            Some(s)
        } else if t.is_ascii_uppercase() {
            let n = u32::from_be_bytes(data.get(*p..*p + 4)?.try_into().ok()?) as usize;
            let s = data.get(*p + 4..*p + 4 + n)?;
            *p += 4 + n;
            Some(s)
        } else {
            let end = *p + data[*p..].iter().position(|&b| b == 0).unwrap_or(data.len() - *p);
            let s = &data[*p..end];
            *p = (end + 1).min(data.len());
            Some(s)
        }
    };
    match sametypes {
        Some(types) => {
            for (i, &t) in types.iter().enumerate() {
                match take(&mut p, t, i + 1 == types.len()) {
                    Some(s) => out.push((t, s)),
                    None => break,
                }
            }
        }
        None => {
            while p < data.len() {
                let t = data[p];
                p += 1;
                match take(&mut p, t, false) {
                    Some(s) => out.push((t, s)),
                    None => break,
                }
            }
        }
    }
    out
}

/// Nội dung đã chuẩn hoá của một mục: (MARK_* hoặc 0 nếu chữ thường, thân bài).
fn entry_body(data: &[u8], sametypes: Option<&[u8]>) -> Option<(u8, String)> {
    let parts = parts(data, sametypes);
    let kind = parts.iter().find_map(|&(t, _)| match t {
        b'h' => Some(MARK_HTML),
        b'x' => Some(MARK_XDXF),
        b'g' => Some(MARK_PANGO),
        _ => None,
    });
    let mut pieces = Vec::new();
    for (t, s) in parts {
        let s = String::from_utf8_lossy(s).replace("\r\n", "\n");
        let s = s.trim();
        if s.is_empty() {
            continue;
        }
        let piece = match (t, kind) {
            (b'h' | b'x' | b'g', _) => s.to_string(),
            (b't', None) => format!("/{s}/"),
            (b't', Some(k)) => esc_markup(&format!("/{s}/"), k),
            (b'm' | b'l' | b'y' | b'k' | b'w', None) => s.to_string(),
            (b'm' | b'l' | b'y' | b'k' | b'w', Some(k)) => esc_markup(s, k),
            _ => continue, // tài nguyên, âm thanh, hình ảnh...
        };
        pieces.push(piece);
    }
    if pieces.is_empty() {
        return None;
    }
    let sep = match kind {
        Some(MARK_HTML) => "<br>",
        _ => "\n",
    };
    Some((kind.unwrap_or(0), pieces.join(sep)))
}

/// Mục chữ thường có dòng tiêu đề "@từ" (không phải mục chuyên ngành của OVDP).
fn is_general(t: &str) -> bool {
    t.starts_with('@') && !t.starts_with("@Chuyên ngành") && !t.starts_with("@Lĩnh vực")
}

/// Gộp các mục cùng khoá thành một nội dung.
fn merge(mut items: Vec<(String, u8, String)>) -> String {
    let markup = items.iter().map(|i| i.1).find(|&k| k != 0);
    match markup {
        None => {
            // Nghĩa thông dụng trước, nghĩa chuyên ngành sau (OVDP).
            items.sort_by_key(|i| !is_general(&i.2));
            items.dedup_by(|a, b| a.2 == b.2);
            let mut out = String::new();
            if !is_general(&items[0].2) {
                out.push('@');
                out.push_str(&items[0].0);
                out.push('\n');
            }
            let texts: Vec<&str> = items.iter().map(|i| i.2.as_str()).collect();
            out.push_str(&texts.join("\n"));
            out
        }
        Some(k) => {
            items.dedup_by(|a, b| a.2 == b.2);
            let sep = if k == MARK_HTML { "<br><br>" } else { "\n\n" };
            let bodies: Vec<String> = items
                .iter()
                .map(|i| if i.1 == 0 { esc_markup(&i.2, k) } else { i.2.clone() })
                .collect();
            format!("{}{}\n{}", k as char, items[0].0, bodies.join(sep))
        }
    }
}

/// Chuyển một từ điển StarDict sang evdict. `progress` nhận phần trăm 0..=100.
pub fn convert(
    src: &Source,
    name: Option<&str>,
    level: u8,
    progress: &mut dyn FnMut(u32),
) -> Result<Converted, String> {
    let info = parse_ifo(&src.ifo)?;
    let off_len = if info.offset_bits == 64 { 8 } else { 4 };

    // .idx: từ\0 + offset (32/64 bit) + size (32 bit), big endian.
    let mut idx: Vec<(String, usize, usize)> = Vec::new();
    let mut p = 0;
    let d = &src.idx;
    while p < d.len() {
        let end = p + d[p..].iter().position(|&b| b == 0).ok_or("file .idx bị hỏng")?;
        let rec = d.get(end + 1..end + 1 + off_len + 4).ok_or("file .idx bị cụt")?;
        let off = if off_len == 8 {
            u64::from_be_bytes(rec[..8].try_into().unwrap()) as usize
        } else {
            u32::from_be_bytes(rec[..4].try_into().unwrap()) as usize
        };
        let size = u32::from_be_bytes(rec[off_len..].try_into().unwrap()) as usize;
        idx.push((String::from_utf8_lossy(&d[p..end]).trim().to_string(), off, size));
        p = end + 1 + off_len + 4;
    }
    if idx.is_empty() {
        return Err("từ điển không có mục nào".into());
    }

    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, (word, _, _)) in idx.iter().enumerate() {
        let k = norm_key(word);
        if !k.is_empty() && k.len() <= 200 {
            groups.entry(k).or_default().push(i);
        }
    }

    let mut data: Vec<u8> = Vec::new();
    let mut records: Vec<(String, u32, u32)> = Vec::with_capacity(groups.len());
    let mut range_of: HashMap<usize, (u32, u32)> = HashMap::new();
    let total = groups.len().max(1);
    for (gi, (key, members)) in groups.iter().enumerate() {
        if gi % 4096 == 0 {
            progress((gi * 85 / total) as u32);
        }
        let items: Vec<(String, u8, String)> = members
            .iter()
            .filter_map(|&i| {
                let (word, off, size) = &idx[i];
                let raw = src.dict.get(*off..off + size)?;
                let (kind, body) = entry_body(raw, info.sametypes.as_deref())?;
                Some((word.clone(), kind, body))
            })
            .collect();
        if items.is_empty() {
            continue;
        }
        let text = merge(items);
        let range = (data.len() as u32, text.len() as u32);
        data.extend_from_slice(text.as_bytes());
        records.push((key.clone(), range.0, range.1));
        for &i in members {
            range_of.insert(i, range);
        }
    }

    // .syn: từ đồng nghĩa\0 + chỉ số mục trong .idx (u32 BE). Trỏ thẳng vào dữ liệu có sẵn.
    if let Some(syn) = &src.syn {
        let known: std::collections::HashSet<String> = records.iter().map(|r| r.0.clone()).collect();
        let mut added = std::collections::HashSet::new();
        let mut p = 0;
        while p < syn.len() {
            let Some(end) = syn[p..].iter().position(|&b| b == 0).map(|e| p + e) else { break };
            let Some(rec) = syn.get(end + 1..end + 5) else { break };
            let target = u32::from_be_bytes(rec.try_into().unwrap()) as usize;
            let k = norm_key(&String::from_utf8_lossy(&syn[p..end]));
            p = end + 5;
            if k.is_empty() || k.len() > 200 || known.contains(&k) || !added.insert(k.clone()) {
                continue;
            }
            if let Some(&(off, len)) = range_of.get(&target) {
                records.push((k, off, len));
            }
        }
        records.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    }

    progress(85);
    let blocks: Vec<Vec<u8>> = data
        .chunks(BLOCK)
        .map(|c| miniz_oxide::deflate::compress_to_vec(c, level))
        .collect();
    progress(98);

    let name = name.map(str::to_string).unwrap_or(info.name);
    let name_bytes = name.as_bytes();
    let ent_off = (36 + name_bytes.len() + 3) & !3;
    let keys_len: usize = records.iter().map(|r| r.0.len()).sum();
    let keys_off = ent_off + (records.len() + 1) * 12;
    let blk_off = keys_off + keys_len;
    let mut cur = blk_off + (blocks.len() + 1) * 4;

    let mut out = Vec::with_capacity(cur + blocks.iter().map(Vec::len).sum::<usize>());
    let put = |out: &mut Vec<u8>, v: usize| out.extend_from_slice(&(v as u32).to_le_bytes());
    out.extend_from_slice(b"EVD2");
    put(&mut out, records.len());
    put(&mut out, BLOCK);
    put(&mut out, blocks.len());
    put(&mut out, ent_off);
    put(&mut out, keys_off);
    put(&mut out, blk_off);
    put(&mut out, data.len());
    put(&mut out, name_bytes.len());
    out.extend_from_slice(name_bytes);
    out.resize(ent_off, 0);
    let mut key_pos = 0;
    for (k, off, len) in &records {
        put(&mut out, key_pos);
        put(&mut out, *off as usize);
        put(&mut out, *len as usize);
        key_pos += k.len();
    }
    put(&mut out, key_pos);
    put(&mut out, 0);
    put(&mut out, 0);
    for (k, _, _) in &records {
        out.extend_from_slice(k.as_bytes());
    }
    for b in &blocks {
        put(&mut out, cur);
        cur += b.len();
    }
    put(&mut out, cur);
    for b in &blocks {
        out.extend_from_slice(b);
    }
    progress(100);
    Ok(Converted { name, entries: records.len(), bytes: out })
}
