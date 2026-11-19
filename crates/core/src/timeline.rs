//! Years → months → items, newest first, with counts and sizes.
use crate::scan::Library;
use chrono::Datelike;

#[derive(Debug, Clone, PartialEq)]
pub struct Month {
    pub year: i32,
    /// 1..=12
    pub month: u32,
    /// Indices into `Library::items`, oldest first.
    pub items: Vec<usize>,
    pub bytes: u64,
    pub uncertain: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Year {
    pub year: i32,
    /// Newest first.
    pub months: Vec<Month>,
    pub count: usize,
    pub bytes: u64,
}

/// Build the timeline; with `only_uncertain` it holds just the items dated by modified time.
pub fn build(lib: &Library, only_uncertain: bool) -> Vec<Year> {
    let mut years: Vec<Year> = Vec::new();
    // items are sorted by date, so walk them backwards and push months newest first
    for (ix, it) in lib.items.iter().enumerate().rev() {
        if only_uncertain && !it.uncertain() {
            continue;
        }
        let (y, m) = (it.taken.year(), it.taken.month());
        if years.last().is_none_or(|l| l.year != y) {
            years.push(Year { year: y, months: Vec::new(), count: 0, bytes: 0 });
        }
        let year = years.last_mut().unwrap();
        if year.months.last().is_none_or(|l| l.month != m) {
            year.months.push(Month { year: y, month: m, items: Vec::new(), bytes: 0, uncertain: 0 });
        }
        let month = year.months.last_mut().unwrap();
        month.items.push(ix);
        month.bytes += it.size;
        month.uncertain += it.uncertain() as usize;
        year.count += 1;
        year.bytes += it.size;
    }
    for y in &mut years {
        for m in &mut y.months {
            m.items.reverse();
        }
    }
    years
}
