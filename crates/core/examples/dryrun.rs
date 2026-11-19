//! Read-only dry run: scan folders, report dates by source and timings, and build (but never
//! apply) an organization plan into a destination that is only inspected.
//!
//! cargo run --release -p mosaico-core --example dryrun -- ~/Pictures [more folders…] [--dest DIR]
use mosaico_core::plan::{self, Mode, Options, SkipReason};
use mosaico_core::{DateSource, Format, scan, timeline};
use std::path::PathBuf;
use std::time::Instant;

fn main() {
    let mut roots = Vec::new();
    let mut dest = None;
    let mut verbose = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "-v" {
            verbose = true;
        } else if a == "--dest" {
            dest = args.next().map(PathBuf::from);
        } else {
            roots.push(PathBuf::from(a));
        }
    }
    if roots.is_empty() {
        eprintln!("usage: dryrun FOLDER… [--dest DIR] [-v]"); // i18n-ok: developer tool
        std::process::exit(2);
    }
    let t = Instant::now();
    let progress = scan::Progress::default();
    let lib = scan::scan(&roots, &progress).expect("scan");
    let scan_s = t.elapsed().as_secs_f32();
    println!("roots: {:?}", lib.roots); // i18n-ok
    println!("items: {}  ({:.1} MB)  sidecars: {}  folders: {}  errors: {}", lib.items.len(), lib.total_bytes() as f64 / 1e6, lib.sidecars.len(), progress.dirs.into_inner(), lib.errors); // i18n-ok
    println!("scan + dates: {scan_s:.2} s"); // i18n-ok
    for s in DateSource::ALL {
        println!("  date from {:<9} {}", format!("{s:?}"), lib.count_by_source(s)); // i18n-ok
    }
    for f in Format::ALL {
        let n = lib.items.iter().filter(|i| i.format == f).count();
        if n > 0 {
            let previews = lib.items.iter().filter(|i| i.format == f && !i.previews.is_empty()).count();
            let exif = lib.items.iter().filter(|i| i.format == f && i.source == DateSource::Exif).count();
            println!("  {:<5} {n:>7}  with preview {previews:>7}  exif date {exif:>7}", f.label()); // i18n-ok
        }
    }
    if verbose {
        for it in &lib.items {
            println!("  {:?} {} {}", it.source, it.taken, it.path.display()); // i18n-ok
        }
    }
    let tl = timeline::build(&lib, false);
    for y in tl.iter().take(12) {
        println!("  {}  {:>6} items  {:>8.1} MB  {} months", y.year, y.count, y.bytes as f64 / 1e6, y.months.len()); // i18n-ok
    }
    // thumbnail timing on a sample, rendered in memory only (no cache writes)
    let t = Instant::now();
    let sample: Vec<_> = lib.items.iter().take(50).collect();
    let made = sample.iter().enumerate().filter(|(i, it)| mosaico_core::thumb::render(&mosaico_core::thumb::Job::for_item(*i, it)).is_some()).count();
    println!("thumbnails (in memory): {made}/{} in {:.2} s", sample.len(), t.elapsed().as_secs_f32()); // i18n-ok

    let dest = dest.unwrap_or_else(|| std::env::temp_dir().join("mosaico-dryrun-nonexistent"));
    let names = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"].map(String::from);
    let opts = Options { dest_root: dest.clone(), template: plan::DEFAULT_TEMPLATE.into(), month_names: names, mode: Mode::Copy, include_uncertain: true };
    let t = Instant::now();
    let p = plan::build(&lib, &opts, &plan::Progress::default()).expect("plan");
    println!("plan into {} (not applied): {} ops, {:.1} MB, {} folders, {} renamed, identical {}, duplicates {}, in place {} — {:.2} s", dest.display(), p.ops.len(), p.bytes() as f64 / 1e6, p.folders.len(), p.renamed(), p.skipped_for(SkipReason::Identical), p.skipped_for(SkipReason::DuplicateInSources), p.skipped_for(SkipReason::AlreadyInPlace), t.elapsed().as_secs_f32()); // i18n-ok
    for f in p.folders.iter().take(8) {
        println!("  {}  ({} files)", f.dir.display(), f.ops.len()); // i18n-ok
    }
}
