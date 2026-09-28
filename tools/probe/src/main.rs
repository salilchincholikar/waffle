//! Open a file, report load time / memory, optionally save it.
//! cargo run --release -p waffle-probe -- <in> [out]
use std::time::Instant;

fn rss_mb() -> f64 {
    let out = std::process::Command::new("ps").args(["-o", "rss=", "-p", &std::process::id().to_string()]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse::<f64>().unwrap_or(0.0) / 1024.0
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let base = rss_mb();
    let t = Instant::now();
    let mut doc = waffle_io::doc::Doc::open(std::path::Path::new(&args[1])).expect("open");
    let mut first = None;
    while !doc.is_loaded() {
        if first.is_none() {
            let wb = doc.wb.lock().unwrap();
            if wb.sheets[wb.active].row_count() > 40 {
                first = Some(t.elapsed());
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    doc.wait();
    let load = t.elapsed();
    if let Some(e) = doc.take_error() {
        println!("load error: {e}");
    }
    {
        let wb = doc.wb.lock().unwrap();
        for s in &wb.sheets {
            println!(
                "sheet {:?}: {} rows x {} cols, formulas {}, merges {}",
                s.name,
                s.row_count(),
                s.col_count(),
                s.formulas.len(),
                s.grid.merges.len()
            );
        }
        println!("heap estimate: {:.1} MB", wb.heap_bytes() as f64 / 1048576.0);
    }
    println!("first rows: {:?}  full load: {:?}  RSS: {:.1} MB (baseline {:.1})", first, load, rss_mb(), base);
    if let Some(out) = args.get(2) {
        let t = Instant::now();
        doc.save(std::path::Path::new(out), out.ends_with(".csv") || out.ends_with(".tsv") || out.ends_with(".txt")).expect("save");
        println!("saved in {:?}  RSS: {:.1} MB", t.elapsed(), rss_mb());
    }
}
