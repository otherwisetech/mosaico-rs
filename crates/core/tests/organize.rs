//! Scan → plan → apply → undo against temporary folders.
use mosaico_core::apply::{self, Failure, Kept};
use mosaico_core::fixtures;
use mosaico_core::journal::{self, Record, Writer};
use mosaico_core::plan::{self, Mode, Options, SkipReason};
use mosaico_core::scan::{self, Library};
use mosaico_core::{DateSource, Plan};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;

const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

fn months() -> [String; 12] {
    MONTHS.map(String::from)
}

/// A real JPEG whose EXIF DateTimeOriginal is `date` (`2023:05:14 10:22:33`); `seed` varies
/// the pixels so different seeds give different content.
fn photo(date: &str, seed: u32) -> Vec<u8> {
    let exif = fixtures::exif_block(Some(date), None, None, 1, None);
    fixtures::jpeg_with_exif(&exif, 8 + seed, 8)
}

fn write(p: &Path, bytes: &[u8]) -> PathBuf {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, bytes).unwrap();
    p.to_path_buf()
}

fn scan(roots: &[&Path]) -> Library {
    scan::scan(&roots.iter().map(|p| p.to_path_buf()).collect::<Vec<_>>(), &scan::Progress::default()).unwrap()
}

fn opts(dest: &Path, mode: Mode) -> Options {
    Options { dest_root: dest.to_path_buf(), template: plan::DEFAULT_TEMPLATE.into(), month_names: months(), mode, include_uncertain: true }
}

fn make_plan(lib: &Library, o: &Options) -> Plan {
    plan::build(lib, o, &plan::Progress::default()).unwrap()
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir).map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    v.sort();
    v
}

/// Every file under `dir`, relative, sorted.
fn tree(dir: &Path) -> Vec<String> {
    fn walk(base: &Path, d: &Path, out: &mut Vec<String>) {
        for e in fs::read_dir(d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(base, &p, out);
            } else {
                let rel = p.strip_prefix(base).unwrap();
                out.push(rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/"));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

#[test]
fn scan_skips_hidden_bundles_and_libraries() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("src");
    write(&src.join("ok/a.jpg"), &photo("2023:05:14 10:22:33", 0));
    write(&src.join(".hidden/b.jpg"), &photo("2023:05:14 10:22:33", 1));
    write(&src.join(".c.jpg"), &photo("2023:05:14 10:22:33", 2));
    write(&src.join("Photos Library.photoslibrary/originals/d.jpg"), &photo("2023:05:14 10:22:33", 3));
    write(&src.join("Some.app/Contents/e.png"), b"x");
    write(&src.join("notes.txt"), b"x");
    write(&src.join("ok/a.xmp"), b"<x/>");
    // nested roots are scanned once
    let lib = scan(&[&src, &src.join("ok")]);
    assert_eq!(lib.items.len(), 1);
    assert!(lib.items[0].path.ends_with("ok/a.jpg"));
    assert_eq!(lib.items[0].source, DateSource::Exif);
    assert_eq!(lib.sidecars.len(), 1);
    assert_eq!(lib.roots.len(), 1);
}

#[test]
fn dates_from_each_source_in_a_scan() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("src");
    write(&src.join("exif.jpg"), &photo("2021:03:04 05:06:07", 0));
    write(&src.join("clip.mov"), &fixtures::bmff::movie(0, Some("2020-01-02T03:04:05+0100")));
    write(&src.join("IMG_20190809_101112.jpg"), b"not really a jpeg");
    write(&src.join("holiday.jpg"), b"no date anywhere");
    let lib = scan(&[&src]);
    let by_name = |n: &str| lib.items.iter().find(|i| i.path.file_name().unwrap() == n).unwrap();
    assert_eq!(by_name("exif.jpg").source, DateSource::Exif);
    assert_eq!(by_name("exif.jpg").taken.to_string(), "2021-03-04 05:06:07");
    assert_eq!(by_name("clip.mov").source, DateSource::Video);
    assert_eq!(by_name("clip.mov").taken.to_string(), "2020-01-02 03:04:05");
    assert_eq!(by_name("IMG_20190809_101112.jpg").source, DateSource::Filename);
    assert!(by_name("holiday.jpg").uncertain());
    // sorted oldest first; the timeline is newest first with the uncertain filter
    let tl = mosaico_core::timeline::build(&lib, false);
    assert_eq!(tl.iter().map(|y| y.count).sum::<usize>(), 4);
    assert!(tl.windows(2).all(|w| w[0].year > w[1].year));
    let unc = mosaico_core::timeline::build(&lib, true);
    assert_eq!(unc.iter().map(|y| y.count).sum::<usize>(), 1);
}

#[test]
fn template_rendering() {
    let d = chrono::NaiveDate::from_ymd_opt(2023, 5, 4).unwrap().and_hms_opt(1, 2, 3).unwrap();
    let m = months();
    assert_eq!(plan::render_template(plan::DEFAULT_TEMPLATE, d, &m).unwrap(), PathBuf::from("2023/2023-05 May"));
    assert_eq!(plan::render_template("{year}/{month}/{day}", d, &m).unwrap(), PathBuf::from("2023/05/04"));
    assert_eq!(plan::render_template("/../{year}//x:y/", d, &m).unwrap(), PathBuf::from("2023/x-y"));
    assert_eq!(plan::render_template("{yaer}", d, &m), Err(plan::TemplateError::UnknownToken));
    assert_eq!(plan::render_template("{year", d, &m), Err(plan::TemplateError::Unbalanced));
    assert_eq!(plan::render_template("../..", d, &m), Err(plan::TemplateError::Empty));
}

#[test]
fn collisions_identical_and_duplicates() {
    let t = tempfile::tempdir().unwrap();
    let (src, dest) = (t.path().join("src"), t.path().join("dest"));
    let d = "2023:05:14 10:22:33";
    write(&src.join("cam1/IMG_0001.JPG"), &photo(d, 1));
    write(&src.join("cam2/IMG_0001.JPG"), &photo(d, 2));
    // same content as cam1 in a third folder: planned once
    write(&src.join("backup/IMG_0001.JPG"), &photo(d, 1));
    // already at the destination: identical → skipped; different → renamed around
    write(&src.join("cam1/IMG_0002.JPG"), &photo(d, 3));
    write(&dest.join("2023/2023-05 May/IMG_0002.JPG"), &photo(d, 3));
    write(&src.join("cam1/IMG_0003.JPG"), &photo(d, 4));
    let squatter = write(&dest.join("2023/2023-05 May/IMG_0003.JPG"), b"someone else's file");
    let lib = scan(&[&src]);
    let p = make_plan(&lib, &opts(&dest, Mode::Copy));
    let month = fs::canonicalize(&dest).unwrap().join("2023/2023-05 May");
    let dsts: Vec<String> = p.ops.iter().map(|o| o.dst.strip_prefix(&month).unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(dsts, ["IMG_0001 (2).JPG", "IMG_0001.JPG", "IMG_0003 (2).JPG"]);
    assert_eq!(p.renamed(), 2);
    assert_eq!(p.skipped_for(SkipReason::Identical), 1);
    assert_eq!(p.skipped_for(SkipReason::DuplicateInSources), 1);
    assert_eq!(p.folders.len(), 1);
    assert_eq!(p.folders[0].dir, PathBuf::from("2023/2023-05 May"));
    // planning wrote nothing
    assert_eq!(tree(&dest), ["2023/2023-05 May/IMG_0002.JPG", "2023/2023-05 May/IMG_0003.JPG"]);
    let r = apply::apply(&p, &t.path().join("journals"), &apply::Progress::default()).unwrap();
    assert_eq!((r.done, r.failed.len()), (3, 0));
    assert_eq!(fs::read(&squatter).unwrap(), b"someone else's file");
}

#[test]
fn raw_pairs_and_sidecars_travel_together() {
    let t = tempfile::tempdir().unwrap();
    let (src, dest) = (t.path().join("src"), t.path().join("dest"));
    // the RAW has no readable date; its JPG twin does, so the whole unit uses it
    write(&src.join("IMG_0007.CR2"), b"raw bytes without exif");
    write(&src.join("IMG_0007.JPG"), &photo("2022:12:31 23:59:59", 0));
    write(&src.join("IMG_0007.xmp"), b"<xmp/>");
    write(&src.join("IMG_0007.CR2.xmp"), b"<xmp raw/>");
    write(&src.join("IMG_0007.AAE"), b"<aae/>");
    write(&src.join("lonely.xmp"), b"<xmp/>");
    // a different photo already owns the JPG's name: every member moves to " (2)"
    write(&dest.join("2022/2022-12 December/IMG_0007.JPG"), &photo("2022:12:31 23:59:59", 9));
    let lib = scan(&[&src]);
    let units = plan::units(&lib);
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].members.len(), 5);
    assert_eq!(units[0].source, DateSource::Exif);
    let p = make_plan(&lib, &opts(&dest, Mode::Copy));
    let mut got: Vec<String> = p.ops.iter().map(|o| o.dst.file_name().unwrap().to_string_lossy().into_owned()).collect();
    got.sort();
    assert_eq!(got, ["IMG_0007 (2).AAE", "IMG_0007 (2).CR2", "IMG_0007 (2).CR2.xmp", "IMG_0007 (2).JPG", "IMG_0007 (2).xmp"]);
    assert!(p.ops.iter().all(|o| o.dst.parent().unwrap().ends_with("2022/2022-12 December")));
}

#[test]
fn copy_apply_then_undo() {
    let t = tempfile::tempdir().unwrap();
    let (src, dest, jdir) = (t.path().join("src"), t.path().join("dest"), t.path().join("journals"));
    let a = write(&src.join("a.jpg"), &photo("2023:05:14 10:22:33", 1));
    let b = write(&src.join("sub/b.jpg"), &photo("2021:01:02 03:04:05", 2));
    let before = fs::metadata(&a).unwrap().modified().unwrap();
    let lib = scan(&[&src]);
    let p = make_plan(&lib, &opts(&dest, Mode::Copy));
    let r = apply::apply(&p, &jdir, &apply::Progress::default()).unwrap();
    assert_eq!((r.done, r.failed.len(), r.cancelled), (2, 0, false));
    assert_eq!(tree(&dest), ["2021/2021-01 January/b.jpg", "2023/2023-05 May/a.jpg"]);
    // sources untouched, copies identical, modified time kept, no temp files left
    assert!(a.is_file() && b.is_file());
    let copy = dest.join("2023/2023-05 May/a.jpg");
    assert_eq!(fs::read(&copy).unwrap(), fs::read(&a).unwrap());
    assert_eq!(fs::metadata(&copy).unwrap().modified().unwrap(), before);
    // a second plan finds everything already there
    let again = make_plan(&scan(&[&src]), &opts(&dest, Mode::Copy));
    assert!(again.ops.is_empty());
    assert_eq!(again.skipped_for(SkipReason::Identical), 2);
    // undo removes exactly the copies and the folders it created
    let last = journal::latest_undoable(&jdir).unwrap();
    assert_eq!((last.placed, last.mode), (2, Mode::Copy));
    let u = apply::undo(&last.path, &apply::Progress::default()).unwrap();
    assert_eq!((u.restored, u.kept.len()), (2, 0));
    assert!(names(&dest).is_empty(), "{:?}", tree(&dest));
    assert!(a.is_file() && b.is_file());
    assert!(journal::latest_undoable(&jdir).is_none());
}

#[test]
fn move_apply_then_undo() {
    let t = tempfile::tempdir().unwrap();
    let (src, dest, jdir) = (t.path().join("src"), t.path().join("dest"), t.path().join("journals"));
    let a_bytes = photo("2023:05:14 10:22:33", 1);
    let a = write(&src.join("a.jpg"), &a_bytes);
    write(&src.join("a.xmp"), b"<xmp/>");
    let lib = scan(&[&src]);
    let p = make_plan(&lib, &opts(&dest, Mode::Move));
    assert_eq!(p.mode, Mode::Move);
    let r = apply::apply(&p, &jdir, &apply::Progress::default()).unwrap();
    assert_eq!((r.done, r.failed.len()), (2, 0));
    assert!(!a.exists());
    assert_eq!(tree(&dest), ["2023/2023-05 May/a.jpg", "2023/2023-05 May/a.xmp"]);
    let u = apply::undo(&journal::latest_undoable(&jdir).unwrap().path, &apply::Progress::default()).unwrap();
    assert_eq!(u.restored, 2);
    assert_eq!(fs::read(&a).unwrap(), a_bytes);
    assert_eq!(names(&src), ["a.jpg", "a.xmp"]);
    assert!(names(&dest).is_empty());
}

#[test]
fn apply_never_overwrites_a_file_that_appeared_later() {
    let t = tempfile::tempdir().unwrap();
    let (src, dest) = (t.path().join("src"), t.path().join("dest"));
    write(&src.join("a.jpg"), &photo("2023:05:14 10:22:33", 1));
    let p = make_plan(&scan(&[&src]), &opts(&dest, Mode::Move));
    let late = write(&p.ops[0].dst, b"arrived after planning");
    let r = apply::apply(&p, &t.path().join("j"), &apply::Progress::default()).unwrap();
    assert_eq!(r.failed, vec![(p.ops[0].src.clone(), Failure::DestinationExists)]);
    assert_eq!(fs::read(&late).unwrap(), b"arrived after planning");
    assert!(p.ops[0].src.exists(), "a failed move must keep its source");
}

#[test]
fn uncertain_dates_can_be_left_out() {
    let t = tempfile::tempdir().unwrap();
    let (src, dest) = (t.path().join("src"), t.path().join("dest"));
    write(&src.join("holiday.jpg"), b"no date");
    write(&src.join("a.jpg"), &photo("2023:05:14 10:22:33", 1));
    let lib = scan(&[&src]);
    let p = make_plan(&lib, &Options { include_uncertain: false, ..opts(&dest, Mode::Copy) });
    assert_eq!(p.ops.len(), 1);
    assert_eq!(p.skipped_for(SkipReason::UncertainDate), 1);
}

#[test]
fn files_already_in_place_are_skipped() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("Pictures");
    write(&root.join("2023/2023-05 May/a.jpg"), &photo("2023:05:14 10:22:33", 1));
    let p = make_plan(&scan(&[&root]), &opts(&root, Mode::Move));
    assert!(p.ops.is_empty());
    assert_eq!(p.skipped_for(SkipReason::AlreadyInPlace), 1);
}

#[test]
fn cancelled_apply_is_undoable() {
    let t = tempfile::tempdir().unwrap();
    let (src, dest, jdir) = (t.path().join("src"), t.path().join("dest"), t.path().join("journals"));
    // big enough that the copy loop is still running when we cancel
    let big = vec![7u8; 3 << 20];
    for i in 0..40 {
        write(&src.join(format!("IMG_20230514_1000{i:02}.jpg")), &[big.as_slice(), &[i as u8]].concat());
    }
    let p = make_plan(&scan(&[&src]), &opts(&dest, Mode::Copy));
    assert_eq!(p.ops.len(), 40);
    let progress = apply::Progress::default();
    let r = std::thread::scope(|s| {
        let h = s.spawn(|| apply::apply(&p, &jdir, &progress).unwrap());
        while progress.files.load(Relaxed) < 2 && !h.is_finished() {
            std::thread::yield_now();
        }
        progress.cancel.store(true, Relaxed);
        h.join().unwrap()
    });
    assert!(r.cancelled && r.done >= 2 && r.done < 40, "done {}", r.done);
    assert_eq!(tree(&dest).len(), r.done);
    let u = apply::undo(&r.journal, &apply::Progress::default()).unwrap();
    assert_eq!(u.restored, r.done);
    assert_eq!(names(&src).len(), 40);
    assert!(names(&dest).is_empty());
}

#[test]
fn cancel_before_start_does_nothing() {
    let t = tempfile::tempdir().unwrap();
    let (src, dest) = (t.path().join("src"), t.path().join("dest"));
    write(&src.join("a.jpg"), &photo("2023:05:14 10:22:33", 1));
    let p = make_plan(&scan(&[&src]), &opts(&dest, Mode::Copy));
    let progress = apply::Progress::default();
    progress.cancel.store(true, Relaxed);
    let r = apply::apply(&p, &t.path().join("j"), &progress).unwrap();
    assert!(r.cancelled && r.done == 0 && !dest.exists());
}

#[test]
fn undo_after_crash_and_undo_safety() {
    let t = tempfile::tempdir().unwrap();
    let (src, dest, jdir) = (t.path().join("src"), t.path().join("dest"), t.path().join("journals"));
    let a = write(&src.join("a.jpg"), &photo("2023:05:14 10:22:33", 1));
    let b = write(&src.join("b.jpg"), &photo("2023:05:14 10:22:34", 2));
    let c = write(&src.join("c.jpg"), &photo("2023:05:14 10:22:35", 3));
    let d = write(&src.join("d.jpg"), &photo("2023:05:14 10:22:36", 4));
    let month = dest.join("2023/2023-05 May");
    fs::create_dir_all(&month).unwrap();
    // hand-written journal of a copy that crashed at different points:
    let mut j = Writer::create(&jdir).unwrap();
    j.write(&Record::Header { version: journal::VERSION, mode: Mode::Copy, dest_root: dest.clone(), created: 0, ops: 4 }).unwrap();
    // a: died while writing the temporary file
    let a_tmp = write(&month.join(".a.jpg.mosaico-part"), b"partial");
    j.write(&Record::Begin { i: 0, src: a.clone(), dst: month.join("a.jpg"), tmp: a_tmp.clone() }).unwrap();
    // b: placed, but the Placed record never made it
    let b_dst = write(&month.join("b.jpg"), &fs::read(&b).unwrap());
    j.write(&Record::Begin { i: 1, src: b.clone(), dst: b_dst.clone(), tmp: month.join(".b.jpg.mosaico-part") }).unwrap();
    // c: copied fine, then the user edited the copy: undo must keep it
    let c_dst = write(&month.join("c.jpg"), &fs::read(&c).unwrap());
    let c_hash = mosaico_core::hash::file_hash(&c_dst).unwrap().to_hex().to_string();
    j.write(&Record::Begin { i: 2, src: c.clone(), dst: c_dst.clone(), tmp: month.join(".c.jpg.mosaico-part") }).unwrap();
    j.write(&Record::Placed { i: 2, size: 0, hash: Some(c_hash), linked: false }).unwrap();
    fs::write(&c_dst, b"edited by the user").unwrap();
    // d: copied fine, then the user deleted the original: the copy is now the only one
    let d_dst = write(&month.join("d.jpg"), &fs::read(&d).unwrap());
    let d_hash = mosaico_core::hash::file_hash(&d_dst).unwrap().to_hex().to_string();
    j.write(&Record::Begin { i: 3, src: d.clone(), dst: d_dst.clone(), tmp: month.join(".d.jpg.mosaico-part") }).unwrap();
    j.write(&Record::Placed { i: 3, size: 0, hash: Some(d_hash), linked: false }).unwrap();
    j.write(&Record::Done { i: 3 }).unwrap();
    fs::remove_file(&d).unwrap();
    let path = j.path.clone();
    drop(j);
    // a torn last line from the crash is ignored
    fs::OpenOptions::new().append(true).open(&path).and_then(|mut f| std::io::Write::write_all(&mut f, b"{\"t\":\"beg")).unwrap();

    let u = apply::undo(&path, &apply::Progress::default()).unwrap();
    assert!(!a_tmp.exists());
    assert!(!b_dst.exists() && b.exists());
    assert_eq!(fs::read(&c_dst).unwrap(), b"edited by the user");
    assert!(d_dst.exists(), "the only remaining copy must never be removed");
    assert_eq!(u.restored, 1);
    let mut kept: Vec<_> = u.kept.iter().map(|(_, k)| k.clone()).collect();
    kept.sort_by_key(|k| format!("{k:?}"));
    assert_eq!(kept, [Kept::Changed, Kept::SourceMissing]);
    assert!(a.exists() && c.exists());
}

#[cfg(target_os = "macos")]
#[test]
fn macos_packages_without_extension_are_skipped() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("src");
    write(&src.join("Photo Booth Library/Pictures/a.jpg"), &photo("2023:05:14 10:22:33", 1));
    write(&src.join("plain/b.jpg"), &photo("2023:05:14 10:22:33", 2));
    // Finder flags with kHasBundle (0x2000), as Photo Booth sets it
    let info = format!("{}20{}", "00".repeat(8), "00".repeat(23));
    let ok = std::process::Command::new("xattr").args(["-wx", "com.apple.FinderInfo", &info]).arg(src.join("Photo Booth Library")).status().unwrap();
    assert!(ok.success());
    let lib = scan(&[&src]);
    assert_eq!(lib.items.len(), 1);
    assert!(lib.items[0].path.ends_with("plain/b.jpg"));
}
