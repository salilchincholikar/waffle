//! An open document: format detection and background loading.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::workbook::Workbook;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Xlsx,
    Xlsm,
    Csv,
    Legacy,
}

pub struct Doc {
    pub wb: Arc<Mutex<Workbook>>,
    pub format: Format,
    pub path: PathBuf,
    progress: Arc<Vec<AtomicU32>>,
    cancel: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    threads: Vec<JoinHandle<()>>,
}

fn sniff_format(path: &Path) -> std::io::Result<Format> {
    let mut head = [0u8; 8];
    let n = File::open(path)?.read(&mut head)?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if n >= 4 && head[..4] == *b"PK\x03\x04" {
        // Zip: xlsx/xlsm unless it's xlsb/ods.
        let zip = zip::ZipArchive::new(File::open(path)?).map_err(std::io::Error::other)?;
        let names: Vec<&str> = zip.file_names().collect();
        if names.iter().any(|n| n.ends_with(".bin") && n.starts_with("xl/workbook")) || names.contains(&"content.xml") {
            return Ok(Format::Legacy);
        }
        return Ok(if ext == "xlsm" || names.iter().any(|n| n.ends_with("vbaProject.bin")) { Format::Xlsm } else { Format::Xlsx });
    }
    if n >= 8 && head == [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1] {
        return Ok(Format::Legacy);
    }
    Ok(Format::Csv)
}

impl Doc {
    pub fn open(path: &Path) -> std::io::Result<Doc> {
        let format = sniff_format(path)?;
        let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Sheet1").to_string();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        let cancel = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None));
        let mut threads = Vec::new();

        let (wb, progress) = match format {
            Format::Xlsx | Format::Xlsm => {
                let file = File::open(path)?;
                let (wb, jobs) = crate::xlsx::read::open(file.try_clone()?, format == Format::Xlsm)?;
                let progress: Arc<Vec<AtomicU32>> = Arc::new((0..wb.sheets.len()).map(|_| AtomicU32::new(0)).collect());
                let wb = Arc::new(Mutex::new(wb));
                let jobs = Arc::new(Mutex::new(jobs.into_iter().collect::<std::collections::VecDeque<_>>()));
                let workers = std::thread::available_parallelism().map_or(2, |n| n.get()).min(4).min(jobs.lock().unwrap().len()).max(1);
                for _ in 0..workers {
                    let (wb, jobs, progress, cancel, error) = (wb.clone(), jobs.clone(), progress.clone(), cancel.clone(), error.clone());
                    let file = file.try_clone()?;
                    threads.push(std::thread::spawn(move || {
                        loop {
                            let Some(job) = jobs.lock().unwrap().pop_front() else { break };
                            let ctl = crate::xlsx::read::LoadCtl { wb: &wb, progress: &progress[job.index], cancel: &cancel };
                            if let Err(e) = crate::xlsx::read::load_sheet(&file, &job, &ctl) {
                                error.lock().unwrap().get_or_insert(e.to_string());
                                // Leave the sheet usable (partially loaded) instead of spinning forever.
                                if let Ok(mut w) = wb.lock() {
                                    w.sheets[job.index].loaded = true;
                                }
                                progress[job.index].store(1000, Ordering::Relaxed);
                            }
                        }
                        // The last worker to finish runs any recalculation the file asked for.
                        if let Ok(mut w) = wb.lock() {
                            w.finish_load();
                        }
                    }));
                }
                (wb, progress)
            }
            Format::Csv => {
                let file = File::open(path)?;
                let (wb, _) = crate::csv::open(file, &ext, &name)?;
                let wb = Arc::new(Mutex::new(wb));
                let progress: Arc<Vec<AtomicU32>> = Arc::new(vec![AtomicU32::new(0)]);
                let (w, p, c, e) = (wb.clone(), progress.clone(), cancel.clone(), error.clone());
                threads.push(std::thread::spawn(move || {
                    let ctl = crate::csv::LoadCtl { wb: &w, progress: &p[0], cancel: &c };
                    if let Err(err) = crate::csv::load(&ctl) {
                        e.lock().unwrap().get_or_insert(err.to_string());
                        w.lock().unwrap().sheets[0].loaded = true;
                        p[0].store(1000, Ordering::Relaxed);
                    }
                }));
                (wb, progress)
            }
            Format::Legacy => {
                let wb = crate::legacy::open(path, &ext)?;
                let wb = Arc::new(Mutex::new(wb));
                let progress: Arc<Vec<AtomicU32>> = Arc::new(vec![AtomicU32::new(0)]);
                let (w, p, c, e) = (wb.clone(), progress.clone(), cancel.clone(), error.clone());
                let path = path.to_path_buf();
                threads.push(std::thread::spawn(move || {
                    let ctl = crate::xlsx::read::LoadCtl { wb: &w, progress: &p[0], cancel: &c };
                    if let Err(err) = crate::legacy::load(&path, &ctl) {
                        e.lock().unwrap().get_or_insert(err.to_string());
                        for s in &mut w.lock().unwrap().sheets {
                            s.loaded = true;
                        }
                        p[0].store(1000, Ordering::Relaxed);
                    }
                }));
                (wb, progress)
            }
        };
        Ok(Doc { wb, format, path: path.to_path_buf(), progress, cancel, error, threads })
    }

    /// A new, empty workbook.
    pub fn new_empty() -> Doc {
        let styles = crate::styles::Styles::minimal();
        let sheet = crate::sheet::Sheet::new("Sheet1", crate::strings::SheetStrings::new(Arc::new(crate::strings::StringPool::new())));
        let wb = Workbook::new(vec![sheet], styles, crate::workbook::Source::New);
        Doc {
            wb: Arc::new(Mutex::new(wb)),
            format: Format::Xlsx,
            path: PathBuf::new(),
            progress: Arc::new(vec![AtomicU32::new(1000)]),
            cancel: Arc::new(AtomicBool::new(false)),
            error: Arc::new(Mutex::new(None)),
            threads: Vec::new(),
        }
    }

    /// 0…1000 across all sheets.
    pub fn progress(&self) -> u32 {
        let n = self.progress.len().max(1) as u32;
        self.progress.iter().map(|p| p.load(Ordering::Relaxed)).sum::<u32>() / n
    }

    pub fn is_loaded(&self) -> bool {
        self.progress() >= 1000 && self.wb.lock().unwrap().loaded()
    }

    pub fn take_error(&self) -> Option<String> {
        self.error.lock().unwrap().take()
    }

    pub fn wait(&mut self) {
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }

    pub fn save(&self, path: &Path, as_csv: bool) -> std::io::Result<()> {
        let wb = self.wb.lock().unwrap();
        if !wb.loaded() {
            return Err(std::io::Error::other("The file is still loading."));
        }
        if as_csv {
            // Saving to a different text format (e.g. .csv → .tsv) uses that format's dialect.
            let ext = |p: &Path| p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            let dialect = (ext(path) != ext(&self.path) || !wb.is_csv()).then(|| crate::csv::Dialect::default_for(&ext(path)));
            crate::csv::save(&wb, wb.active, path, dialect)
        } else {
            crate::xlsx::write::save(&wb, path)
        }
    }
}

impl Drop for Doc {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}
