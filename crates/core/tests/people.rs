//! Person tags (`people.json`) and the timeline filtered by them, on temp folders.
use mosaico_core::people::Tags;
use mosaico_core::{fixtures, scan, timeline};
use std::fs;
use std::path::Path;

const ANA: &str = "6f0c2c1e-0000-4000-8000-000000000001";
const LUIS: &str = "6f0c2c1e-0000-4000-8000-000000000002";

fn photo(p: &Path, date: &str, seed: u32) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    let exif = fixtures::exif_block(Some(date), None, None, 1, None);
    fs::write(p, fixtures::jpeg_with_exif(&exif, 8 + seed, 8)).unwrap();
}

#[test]
fn folder_tags_apply_to_everything_under_them() {
    let t = tempfile::tempdir().unwrap();
    let trip = t.path().join("trip");
    let mut tags = Tags::default();
    assert!(tags.add(&trip, ANA, "Ana"));
    assert!(!tags.add(&trip, ANA, "Ana"), "same tag twice");
    assert!(tags.add(&trip.join("beach.jpg"), LUIS, "Luis"));
    assert!(tags.add(&trip.join("beach.jpg"), ANA, "Ana"));
    let of = tags.of_item(&trip.join("beach.jpg"));
    assert_eq!(of.iter().map(|(i, _)| i.as_str()).collect::<Vec<_>>(), [LUIS, ANA]);
    assert_eq!(of[1].1, trip.join("beach.jpg"), "the nearest tag wins");
    assert!(tags.has(&trip.join("day2/x.jpg"), ANA));
    assert!(!tags.has(&trip.join("day2/x.jpg"), LUIS));
    assert!(!tags.has(&t.path().join("other/x.jpg"), ANA));
    assert_eq!(tags.people(), [(ANA.to_string(), "Ana".to_string()), (LUIS.to_string(), "Luis".to_string())]);
    assert!(tags.remove(&trip, ANA));
    assert!(!tags.has(&trip.join("day2/x.jpg"), ANA));
    assert!(tags.remove(&trip.join("beach.jpg"), ANA) && tags.remove(&trip.join("beach.jpg"), LUIS));
    assert!(tags.tags.is_empty());
}

#[test]
fn saves_atomically_and_never_reads_a_broken_file_as_empty() {
    let t = tempfile::tempdir().unwrap();
    let file = t.path().join("data/people.json");
    assert_eq!(Tags::load(&file).unwrap(), Tags { schema: 1, ..Default::default() });
    let mut tags = Tags::default();
    tags.add(&t.path().join("a.jpg"), ANA, "Ana");
    tags.save(&file).unwrap();
    assert_eq!(Tags::load(&file).unwrap().people(), tags.people());
    assert!(!file.with_extension("json.tmp").exists());
    fs::write(&file, "{ not json").unwrap();
    assert!(Tags::load(&file).is_err());
    fs::write(&file, r#"{"schema": 9, "tags": {}}"#).unwrap();
    assert!(Tags::load(&file).is_err());
}

#[test]
fn merged_people_keep_their_tags() {
    let mut tags = Tags::default();
    tags.add(Path::new("/p/a.jpg"), ANA, "Ana");
    tags.add(Path::new("/p/b.jpg"), ANA, "Ana");
    tags.add(Path::new("/p/b.jpg"), LUIS, "Luis");
    assert!(tags.repoint(ANA, LUIS, Some("Luis Ana")));
    assert_eq!(tags.direct(Path::new("/p/a.jpg")), [LUIS]);
    assert_eq!(tags.direct(Path::new("/p/b.jpg")), [LUIS], "no duplicate after the merge");
    assert_eq!(tags.name(LUIS), Some("Luis Ana"));
    assert_eq!(tags.name(ANA), None);
    assert!(!tags.repoint(LUIS, LUIS, Some("Luis Ana")), "nothing to change");
}

#[test]
fn timeline_filtered_by_person() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("src");
    photo(&root.join("trip/a.jpg"), "2023:05:14 10:00:00", 1);
    photo(&root.join("trip/b.jpg"), "2023:06:01 10:00:00", 2);
    photo(&root.join("home/c.jpg"), "2024:01:02 10:00:00", 3);
    let lib = scan::scan(std::slice::from_ref(&root), &scan::Progress::default()).unwrap();
    assert_eq!(lib.items.len(), 3);
    let mut tags = Tags::default();
    // library paths are canonical (macOS: /private/var/…)
    let trip = lib.items.iter().find(|i| i.path.ends_with("trip/a.jpg")).unwrap().path.parent().unwrap().to_path_buf();
    tags.add(&trip, ANA, "Ana");
    let years = timeline::build_with(&lib, |it| tags.has(&it.path, ANA));
    assert_eq!(years.len(), 1);
    assert_eq!(years[0].year, 2023);
    assert_eq!(years[0].count, 2);
    assert_eq!(timeline::build(&lib, false).iter().map(|y| y.count).sum::<usize>(), 3);
}
