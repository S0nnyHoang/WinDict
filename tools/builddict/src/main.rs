//! Tạo dữ liệu có sẵn (dist/evdict.dat) từ một từ điển StarDict.
//! Dùng chung bộ chuyển đổi với tính năng import trong app (evdict/src/stardict.rs).

#[path = "../../../evdict/src/key.rs"]
mod key;
#[path = "../../../evdict/src/stardict.rs"]
mod stardict;

use std::fs;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: builddict <stardict base path, without extension> <out.dat> [name]");
        std::process::exit(2);
    }
    let base = &args[1];
    let read = |ext: &str| fs::read(format!("{base}{ext}"));
    let src = stardict::Source {
        ifo: read(".ifo").expect("read .ifo"),
        idx: stardict::maybe_gunzip(read(".idx").expect("read .idx")).unwrap(),
        dict: stardict::maybe_gunzip(read(".dict.dz").or_else(|_| read(".dict")).expect("read .dict")).unwrap(),
        syn: read(".syn").ok(),
    };
    let conv = stardict::convert(&src, args.get(3).map(String::as_str), 9, &mut |_| {}).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1);
    });
    fs::write(&args[2], &conv.bytes).expect("write output");
    println!(
        "\"{}\": {} words, output {:.1} MB",
        conv.name,
        conv.entries,
        conv.bytes.len() as f64 / 1048576.0
    );
}
