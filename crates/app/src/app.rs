//! Window state: choose sources → scan → timeline → plan preview → apply → done (or undo).
use crate::theme::{SANS, Tokens, tokens};
use crate::thumbs::{LruImages, Thumbs};
use crate::ui::{self, icon};
use crate::util::{group, human};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::input::{InputEvent, InputState};
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_component::{IconName, Sizable as _, Theme};
use mosaico_core::apply::{Report, UndoReport};
use mosaico_core::journal::{self, Summary};
use mosaico_core::plan::{self, Mode, Plan};
use mosaico_core::scan::{self, Library};
use mosaico_core::timeline;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

pub(crate) const CONTEXT: &str = "MosaicoApp";
const TICK: Duration = Duration::from_millis(100);

actions!(mosaico, [AddSource, Rescan, CloseOverlay, ShowAbout]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("secondary-o", AddSource, Some(CONTEXT)), KeyBinding::new("secondary-r", Rescan, Some(CONTEXT)), KeyBinding::new("escape", CloseOverlay, Some(CONTEXT))]);
}

pub(crate) fn sync_theme(window: Option<&mut Window>, cx: &mut App) {
    Theme::sync_system_appearance(window, cx);
    Theme::global_mut(cx).font_family = SANS.into();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Work {
    Apply(Mode),
    Undo,
}

pub enum Phase {
    Start,
    Scanning { progress: Arc<scan::Progress>, started: Instant },
    Ready,
    Planning { progress: Arc<plan::Progress>, started: Instant },
    Preview,
    Working { work: Work, progress: Arc<mosaico_core::apply::Progress>, files: u64, bytes: u64, started: Instant },
    Done,
    Failed { message: String },
}

pub enum Outcome {
    Applied { report: Report, mode: Mode, dest: PathBuf },
    Undone(UndoReport),
}

/// One row of the grid: a slice of one month's items (the first row also shows the month).
#[derive(Debug, Clone)]
pub struct GridRow {
    pub month: usize,
    pub first: bool,
    pub start: usize,
    pub end: usize,
}

pub struct MosaicoApp {
    pub phase: Phase,
    /// Folders chosen on the start screen.
    pub sources: Vec<PathBuf>,
    pub library: Option<Arc<Library>>,
    pub years: Rc<Vec<timeline::Year>>,
    /// The timeline's months in display order (newest first).
    pub months: Rc<Vec<timeline::Month>>,
    pub rows: Rc<Vec<GridRow>>,
    /// First grid row of each month.
    pub month_row: Vec<usize>,
    pub rows_key: (usize, bool, u64),
    pub lib_gen: u64,
    pub only_uncertain: bool,
    pub selected: Option<usize>,
    pub collapsed_years: HashSet<i32>,
    pub thumbs: Thumbs,
    pub images: Entity<LruImages>,
    pub grid_scroll: UniformListScrollHandle,
    // organize panel
    pub panel: bool,
    pub dest: Option<PathBuf>,
    pub template: Entity<InputState>,
    pub mode: Mode,
    pub include_uncertain: bool,
    pub plan: Option<Arc<Plan>>,
    pub preview_rows: Rc<Vec<crate::organize::PreviewRow>>,
    pub preview_scroll: UniformListScrollHandle,
    pub outcome: Option<Outcome>,
    pub last_journal: Option<Summary>,
    pub work_gen: u64,
    pub recent: Vec<PathBuf>,
    focus: FocusHandle,
    pub about: bool,
    pub logo: Arc<Image>,
    /// Person tags (`people.json`); `tags_ok` is false when the file exists but can't be read.
    pub tags: mosaico_core::people::Tags,
    pub tags_file: Option<PathBuf>,
    pub tags_ok: bool,
    /// Show only the photos of this registry person.
    pub person: Option<String>,
    /// The registry's people; None while the shared registry is off.
    pub people: Option<Vec<(String, String)>>,
    /// Other Otherwise apps installed here, found when About opens.
    pub other_apps: Vec<(otherwise_link::apps::AppSpec, otherwise_link::apps::Installed)>,
    _subs: Vec<Subscription>,
}

impl MosaicoApp {
    pub fn new(initial: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        sync_theme(Some(window), cx);
        let template = cx.new(|cx| InputState::new(window, cx).default_value(plan::DEFAULT_TEMPLATE));
        let subs = vec![
            window.observe_window_appearance(|window, cx| sync_theme(Some(window), cx)),
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    this.refresh_people(cx);
                }
            }),
            cx.subscribe_in(&template, window, |_, _, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.notify();
                }
            }),
        ];
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let mut this = MosaicoApp {
            phase: Phase::Start,
            sources: Vec::new(),
            library: None,
            years: Rc::new(Vec::new()),
            months: Rc::new(Vec::new()),
            rows: Rc::new(Vec::new()),
            month_row: Vec::new(),
            rows_key: (0, false, 0),
            lib_gen: 0,
            only_uncertain: false,
            selected: None,
            collapsed_years: HashSet::new(),
            thumbs: Thumbs::new(),
            images: LruImages::new(600, cx),
            grid_scroll: UniformListScrollHandle::new(),
            panel: false,
            dest: None,
            template,
            mode: Mode::Copy,
            include_uncertain: true,
            plan: None,
            preview_rows: Rc::new(Vec::new()),
            preview_scroll: UniformListScrollHandle::new(),
            outcome: None,
            last_journal: None,
            work_gen: 0,
            recent: crate::settings::recent(),
            focus,
            about: false,
            logo: Arc::new(Image::from_bytes(ImageFormat::Png, crate::ICON_PNG.to_vec())),
            tags: Default::default(),
            tags_file: mosaico_core::people::default_file(),
            tags_ok: true,
            person: None,
            people: None,
            other_apps: Vec::new(),
            _subs: subs,
        };
        if let Some(f) = &this.tags_file {
            match mosaico_core::people::Tags::load(f) {
                Ok(t) => this.tags = t,
                Err(e) => {
                    eprintln!("mosaico: {}: {e}", f.display()); // i18n-ok: developer diagnostic
                    this.tags_ok = false;
                }
            }
        }
        this.refresh_people(cx);
        this.watch_links(window, cx);
        this.refresh_journal();
        this.start_ticker(cx);
        let initial: Vec<PathBuf> = initial.into_iter().filter(|p| p.is_dir()).collect();
        if !initial.is_empty() {
            this.sources = initial;
            this.start_scan(window, cx);
        }
        crate::star::refresh(crate::settings::Settings::load().github_stars, cx);
        this
    }

    /// One timer for the whole window: collects finished thumbnails and repaints live counters.
    fn start_ticker(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let alive = this.update(cx, |this, cx| {
                    let busy = matches!(this.phase, Phase::Scanning { .. } | Phase::Planning { .. } | Phase::Working { .. });
                    if this.thumbs.poll() || busy {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    pub fn refresh_journal(&mut self) {
        self.last_journal = mosaico_core::dirs::journal_dir().and_then(|d| journal::latest_undoable(&d));
    }

    pub fn add_source_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: true, prompt: Some(i18n::t!("app.start.add_prompt").into()) });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                this.update_in(cx, |this, window, cx| {
                    for p in paths {
                        this.add_source(p);
                    }
                    if !matches!(this.phase, Phase::Start) {
                        this.go_start(window, cx);
                    }
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    pub fn add_source(&mut self, p: PathBuf) {
        if !self.sources.contains(&p) {
            self.sources.push(p);
        }
    }

    fn go_start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_running();
        self.phase = Phase::Start;
        self.refresh_journal();
        window.set_window_title(crate::APP_NAME);
        cx.notify();
    }

    pub fn start_scan(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sources.is_empty() {
            return;
        }
        self.cancel_running();
        self.work_gen += 1;
        let gen_ = self.work_gen;
        let progress = Arc::new(scan::Progress::default());
        self.phase = Phase::Scanning { progress: progress.clone(), started: Instant::now() };
        let roots = self.sources.clone();
        let p = progress.clone();
        let task = cx.background_executor().spawn(async move { scan::scan(&roots, &p) });
        cx.spawn_in(window, async move |this, cx| {
            let res = task.await;
            this.update_in(cx, |this, window, cx| {
                if this.work_gen != gen_ {
                    return;
                }
                match res {
                    Ok(lib) => this.loaded(lib, window, cx),
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(e) => this.phase = Phase::Failed { message: i18n::t!("app.error.other", detail = e) },
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn loaded(&mut self, lib: Library, window: &mut Window, cx: &mut Context<Self>) {
        for r in &lib.roots {
            crate::settings::add_recent(r);
        }
        self.recent = crate::settings::recent();
        let title = match lib.roots.as_slice() {
            [one] => format!("{} — {}", display_name(one), crate::APP_NAME),
            _ => crate::APP_NAME.to_string(),
        };
        window.set_window_title(&title);
        self.thumbs.reset();
        self.library = Some(Arc::new(lib));
        self.lib_gen += 1;
        self.selected = None;
        self.plan = None;
        self.rebuild_timeline();
        self.phase = Phase::Ready;
        cx.notify();
    }

    pub fn rebuild_timeline(&mut self) {
        let Some(lib) = &self.library else { return };
        let only_uncertain = self.only_uncertain;
        let years = match &self.person {
            Some(id) => timeline::build_with(lib, |it| self.tags.has(&it.path, id) && (!only_uncertain || it.uncertain())),
            None => timeline::build(lib, only_uncertain),
        };
        self.months = Rc::new(years.iter().flat_map(|y| y.months.iter().cloned()).collect());
        self.years = Rc::new(years);
        self.rows_key = (0, false, 0);
        self.grid_scroll.scroll_to_item(0, ScrollStrategy::Top);
    }

    /// Stop whatever runs in the background (its result will be ignored).
    fn cancel_running(&mut self) {
        match &self.phase {
            Phase::Scanning { progress, .. } => progress.cancel.store(true, Relaxed),
            Phase::Planning { progress, .. } => progress.cancel.store(true, Relaxed),
            _ => return,
        }
        self.work_gen += 1;
    }

    fn stop_scan(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_running();
        self.phase = if self.library.is_some() { Phase::Ready } else { Phase::Start };
        if self.library.is_none() {
            window.set_window_title(crate::APP_NAME);
        }
        cx.notify();
    }

    fn set_language(&mut self, code: &'static str, cx: &mut Context<Self>) {
        if i18n::set_locale(code) {
            let mut s = crate::settings::Settings::load();
            s.language = Some(code.to_string());
            s.save();
            gpui_component::set_locale(code);
            crate::set_app_menus(cx);
            cx.refresh_windows();
        }
    }

    /// Esc: About, then the organize panel, then the preview.
    fn escape(&mut self, cx: &mut Context<Self>) {
        if self.about {
            self.about = false;
        } else if self.panel && matches!(self.phase, Phase::Ready) {
            self.panel = false;
        } else if matches!(self.phase, Phase::Preview) {
            self.phase = Phase::Ready;
        } else {
            return;
        }
        cx.notify();
    }

    fn set_github_stars(&mut self, on: bool, cx: &mut Context<Self>) {
        let mut s = crate::settings::Settings::load();
        s.github_stars = on;
        s.save();
        crate::star::refresh(on, cx);
        cx.notify();
    }

    pub fn show_about(&mut self, cx: &mut Context<Self>) {
        self.about = true;
        self.find_other_apps(cx);
        cx.notify();
    }

    // ── rendering ─────────────────────────────────────────────────────────────────────────────

    fn render_topbar(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let mut left = div().flex().items_center().gap(px(10.)).min_w_0().flex_1();
        left = left.child(div().flex().flex_none().items_center().justify_center().size(px(30.)).rounded(px(8.)).bg(t.brand).child(ui::logo_mark(17., gpui::white())));
        match (&self.library, &self.phase) {
            (Some(lib), Phase::Ready | Phase::Preview | Phase::Planning { .. }) => {
                let summary = i18n::tn!("app.topbar.items", lib.items.len(), count = group(lib.items.len() as u64), size = human(lib.total_bytes()));
                let name = match lib.roots.as_slice() {
                    [one] => display_name(one),
                    many => i18n::tn!("app.topbar.sources", many.len()),
                };
                left = left.child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).truncate().child(name)).child(div().flex_none().text_size(px(12.)).text_color(t.ink_muted).child(summary));
            }
            _ => left = left.child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(crate::APP_NAME)),
        }
        let ready = matches!(self.phase, Phase::Ready);
        let current = i18n::locale();
        let weak = cx.entity().downgrade();
        let stars_on = crate::star::enabled(cx);
        let lang = Button::new("lang").ghost().icon(IconName::Globe).with_size(px(34.)).tooltip(i18n::t!("common.language")).dropdown_menu_with_anchor(Anchor::TopRight, move |mut menu, _, _| {
            menu = menu.label(i18n::t!("common.language"));
            for (code, name) in i18n::available() {
                let w = weak.clone();
                menu = menu.item(PopupMenuItem::new(name).checked(code == current).on_click(move |_, _, cx| {
                    w.update(cx, |this, cx| this.set_language(code, cx)).ok();
                }));
            }
            let w = weak.clone();
            menu = menu.separator().item(PopupMenuItem::new(i18n::t!("app.star.setting")).checked(stars_on).on_click(move |_, _, cx| {
                w.update(cx, |this, cx| this.set_github_stars(!stars_on, cx)).ok();
            }));
            menu = menu.label(i18n::t!("app.star.setting_hint"));
            let w = weak.clone();
            menu.separator().item(PopupMenuItem::new(i18n::t!("app.about.menu", app = crate::APP_NAME)).on_click(move |_, _, cx| {
                w.update(cx, |this, cx| this.show_about(cx)).ok();
            }))
        });
        let uncertain = self.library.as_ref().map(|l| l.count_by_source(mosaico_core::DateSource::Modified)).unwrap_or(0);
        let filter_on = self.only_uncertain;
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .h(px(52.))
            .px(px(12.))
            .border_b_1()
            .border_color(t.border)
            .bg(t.surface)
            .child(left)
            .when(ready && uncertain > 0, |d| {
                let (fg, bg, border) = if filter_on { (t.warn, t.warn_soft, t.warn) } else { (t.ink, t.surface, t.border) };
                d.child(
                    ui::outline_button(t, "filter-uncertain")
                        .border_color(border)
                        .bg(bg)
                        .text_color(fg)
                        .child(icon(IconName::TriangleAlert, 14., fg))
                        .child(i18n::t!("app.topbar.uncertain", n = group(uncertain as u64)))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.only_uncertain = !this.only_uncertain;
                            this.rebuild_timeline();
                            cx.notify();
                        })),
                )
            })
            .when(ready, |d| {
                d.child(ui::filled_button(t, "organize", t.accent).child(icon(IconName::LayoutDashboard, 15., t.on_fill())).child(i18n::t!("app.topbar.organize")).on_click(cx.listener(|this, _, _, cx| {
                    this.panel = !this.panel;
                    cx.notify();
                })))
            })
            .when(ready, |d| d.child(ui::icon_button(t, "rescan", IconName::Redo, true).on_click(cx.listener(|this, _, window, cx| this.start_scan(window, cx)))))
            .when(!matches!(self.phase, Phase::Start | Phase::Working { .. }), |d| {
                d.child(ui::outline_button(t, "sources").child(icon(IconName::FolderOpen, 15., t.ink)).child(i18n::t!("app.topbar.sources_button")).on_click(cx.listener(|this, _, window, cx| this.go_start(window, cx))))
            })
            .child(lang)
    }

    fn render_start(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let sources = self.sources.iter().enumerate().map(|(i, p)| {
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(12.))
                .py(px(7.))
                .child(icon(IconName::Folder, 15., t.accent))
                .child(div().flex().flex_col().flex_1().min_w_0().child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).truncate().child(display_name(p))).child(div().text_size(px(11.)).text_color(t.ink_muted).truncate().child(p.display().to_string())))
                .child(ui::icon_button(t, ("remove-source", i), IconName::Close, true).size(px(26.)).on_click(cx.listener(move |this, _, _, cx| {
                    this.sources.remove(i);
                    cx.notify();
                })))
        });
        let recent = self.recent.iter().filter(|p| !self.sources.contains(p)).enumerate().map(|(i, p)| {
            let p2 = p.clone();
            let hover = t.canvas;
            div()
                .id(("recent", i))
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(12.))
                .py(px(7.))
                .rounded(px(10.))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .child(icon(IconName::Plus, 14., t.ink_muted))
                .child(div().flex().flex_col().min_w_0().child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(display_name(p))).child(div().text_size(px(11.)).text_color(t.ink_muted).truncate().child(p.display().to_string())))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.add_source(p2.clone());
                    cx.notify();
                }))
        });
        let has_recent = self.recent.iter().any(|p| !self.sources.contains(p));
        let can_scan = !self.sources.is_empty();
        let scan_btn = ui::filled_button(t, "scan", if can_scan { t.accent } else { t.border_strong }).child(icon(IconName::Search, 15., t.on_fill())).child(i18n::t!("app.start.scan"));
        div()
            .id("start")
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(20.))
                    .py(px(48.))
                    .px(px(16.))
                    .child(img(self.logo.clone()).size(px(104.)))
                    .child(div().flex().flex_col().items_center().gap(px(6.)).child(div().text_size(px(26.)).font_weight(FontWeight::SEMIBOLD).child(i18n::t!("app.start.title"))).child(div().max_w(px(540.)).text_center().text_size(px(14.)).text_color(t.ink_muted).child(i18n::t!("app.start.subtitle"))))
                    .when(can_scan, |d| d.child(div().flex().flex_col().w(px(480.)).py(px(4.)).rounded(px(14.)).border_1().border_color(t.border).bg(t.surface).child(div().px(px(12.)).pt(px(8.)).pb(px(2.)).child(ui::section_title(t, i18n::t!("app.start.sources")))).children(sources)))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(ui::outline_button(t, "add").child(icon(IconName::FolderOpen, 15., t.ink)).child(if can_scan { i18n::t!("app.start.add_more") } else { i18n::t!("app.start.add") }).on_click(cx.listener(|this, _, window, cx| this.add_source_dialog(window, cx))))
                            .when(can_scan, |d| d.child(scan_btn.on_click(cx.listener(|this, _, window, cx| this.start_scan(window, cx))))),
                    )
                    .when(has_recent, |d| d.child(div().flex().flex_col().w(px(480.)).gap(px(2.)).child(div().px(px(12.)).pb(px(4.)).child(ui::section_title(t, i18n::t!("app.start.recent")))).children(recent)))
                    .children(self.last_journal.as_ref().map(|j| self.render_undo_card(t, j, cx))),
            )
    }

    fn render_undo_card(&self, t: &Tokens, j: &Summary, cx: &mut Context<Self>) -> impl IntoElement {
        let when = chrono_local(j.created);
        let mode = match j.mode {
            Mode::Copy => i18n::t!("app.undo.mode_copy"),
            Mode::Move => i18n::t!("app.undo.mode_move"),
        };
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .w(px(480.))
            .px(px(14.))
            .py(px(10.))
            .rounded(px(14.))
            .border_1()
            .border_color(t.border)
            .bg(t.surface)
            .child(icon(IconName::Undo2, 18., t.ink_muted))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(i18n::t!("app.undo.last_title")))
                    .child(div().text_size(px(11.)).text_color(t.ink_muted).truncate().child(i18n::tn!("app.undo.last_detail", j.placed, when = when, mode = mode, path = j.dest_root.display()))),
            )
            .child(ui::outline_button(t, "undo-last").child(i18n::t!("app.undo.button")).on_click(cx.listener(|this, _, window, cx| this.confirm_undo(window, cx))))
    }

    fn render_scanning(&self, t: &Tokens, p: &scan::Progress, started: Instant, cx: &mut Context<Self>) -> impl IntoElement {
        let secs = started.elapsed().as_secs_f32();
        let stat = |label: String, value: String| div().flex().flex_col().items_center().gap(px(2.)).w(px(130.)).child(div().text_size(px(20.)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink).child(value)).child(div().text_size(px(12.)).text_color(t.ink_muted).child(label));
        let found = p.found.load(Relaxed);
        let read = p.read.load(Relaxed);
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(20.))
            .size_full()
            .child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink).child(i18n::t!("app.scan.title")))
            .child(div().max_w(px(560.)).truncate().text_size(px(13.)).text_color(t.ink_muted).child(self.sources.iter().map(|s| display_name(s)).collect::<Vec<_>>().join(" · ")))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .p(px(16.))
                    .rounded(px(14.))
                    .border_1()
                    .border_color(t.border)
                    .bg(t.surface)
                    .child(stat(i18n::t!("app.scan.found"), group(found)))
                    .child(stat(i18n::t!("app.scan.dated"), group(read)))
                    .child(stat(i18n::t!("app.scan.folders"), group(p.dirs.load(Relaxed))))
                    .child(stat(i18n::t!("app.scan.size"), human(p.bytes.load(Relaxed))))
                    .child(stat(i18n::t!("app.scan.elapsed"), format!("{secs:.1} s"))),
            )
            .child(div().w(px(420.)).child(ui::progress_bar(t, if found == 0 { 0. } else { read as f32 / found as f32 }, t.accent)))
            .child(ui::outline_button(t, "cancel").child(i18n::t!("common.cancel")).on_click(cx.listener(|this, _, window, cx| this.stop_scan(window, cx))))
    }

    fn render_failed(&self, t: &Tokens, message: &str, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(14.))
            .size_full()
            .child(icon(IconName::TriangleAlert, 32., t.danger))
            .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(i18n::t!("app.error.title")))
            .child(div().max_w(px(520.)).text_center().text_size(px(13.)).child(message.to_string()))
            .child(ui::outline_button(t, "back").child(i18n::t!("app.error.back")).on_click(cx.listener(|this, _, window, cx| {
                if this.library.is_some() {
                    this.phase = Phase::Ready;
                    cx.notify();
                } else {
                    this.go_start(window, cx);
                }
            })))
    }

    /// Slim bar at the bottom of every screen: context on the left, About on the right.
    fn render_footer(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let left = match (&self.phase, self.selected.zip(self.library.as_ref())) {
            (Phase::Ready, Some((ix, lib))) => lib.items.get(ix).map(|it| i18n::t!("app.footer.item", date = it.taken.format("%Y-%m-%d %H:%M"), source = source_label(it.source), path = it.path.display())).unwrap_or_default(),
            (Phase::Ready | Phase::Preview, None) => self.library.as_ref().map(|l| l.roots.iter().map(|r| r.display().to_string()).collect::<Vec<_>>().join(" · ")).unwrap_or_default(),
            _ => String::new(),
        };
        let ink = t.ink;
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(12.))
            .h(px(26.))
            .px(px(12.))
            .border_t_1()
            .border_color(t.border)
            .bg(t.surface)
            .text_size(px(11.))
            .text_color(t.ink_muted)
            .child(div().flex_1().min_w_0().truncate().child(left))
            .child(crate::star::footer_link(t, cx))
            .child(
                div()
                    .id("footer-about")
                    .flex_none()
                    .cursor_pointer()
                    .hover(move |s| s.text_color(ink).underline())
                    .child(i18n::t!("app.about.footer", app = crate::APP_NAME, version = crate::VERSION, company = crate::COMPANY))
                    .on_click(cx.listener(|this, _, _, cx| this.show_about(cx))),
            )
    }

    fn render_about(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = t.accent;
        let card = div()
            .id("about-card")
            .relative()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(12.))
            .w(px(420.))
            .px(px(32.))
            .pt(px(32.))
            .pb(px(26.))
            .rounded(px(18.))
            .border_1()
            .border_color(t.border)
            .bg(t.surface)
            .shadow_lg()
            // clicks inside the card must not reach the backdrop
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().absolute().top(px(10.)).right(px(10.)).child(ui::icon_button(t, "about-close", IconName::Close, true).on_click(cx.listener(|this, _, _, cx| {
                this.about = false;
                cx.notify();
            }))))
            .child(img(self.logo.clone()).size(px(88.)))
            .child(div().flex().items_baseline().gap(px(8.)).child(div().text_size(px(22.)).font_weight(FontWeight::SEMIBOLD).child(crate::APP_NAME)).child(div().text_size(px(13.)).text_color(t.ink_muted).child(crate::VERSION)))
            .child(div().text_center().text_size(px(13.)).line_height(px(19.)).text_color(t.ink_muted).child(i18n::t!("app.about.description")))
            .child(div().w_full().h(px(1.)).my(px(4.)).bg(t.border))
            .child(div().flex().flex_col().items_center().gap(px(2.)).child(div().text_size(px(13.)).child(i18n::t!("app.about.by", company = crate::COMPANY))).child(div().text_size(px(12.)).text_color(t.ink_muted).child(i18n::t!("app.about.place"))))
            .child(ui::filled_button(t, "about-site", t.accent).mt(px(4.)).child(i18n::t!("app.about.visit")).on_click(|_, _, cx| cx.open_url(crate::COMPANY_URL)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .child(div().id("about-source").text_size(px(12.)).text_color(accent).cursor_pointer().hover(|s| s.underline()).child(i18n::t!("app.about.source")).on_click(|_, _, cx| cx.open_url(crate::REPO_URL)))
                    .child(crate::star::about_button(t, cx)),
            )
            .child(crate::star::about_ask(t))
            .children(self.render_other_apps(t))
            .child(div().text_size(px(11.)).text_color(t.ink_muted).child(i18n::t!("app.about.license")));
        div()
            .id("about-backdrop")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::black().opacity(if t.dark { 0.55 } else { 0.32 }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.about = false;
                    cx.notify();
                }),
            )
            .child(card)
    }
}

impl Render for MosaicoApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = tokens(cx);
        let body: AnyElement = match &self.phase {
            Phase::Start => self.render_start(&t, cx).into_any_element(),
            Phase::Scanning { progress, started } => {
                let (progress, started) = (progress.clone(), *started);
                self.render_scanning(&t, &progress, started, cx).into_any_element()
            }
            Phase::Ready => self.render_timeline(&t, window, cx).into_any_element(),
            Phase::Planning { progress, started } => {
                let (progress, started) = (progress.clone(), *started);
                self.render_planning(&t, &progress, started, cx).into_any_element()
            }
            Phase::Preview => self.render_preview(&t, cx).into_any_element(),
            Phase::Working { work, progress, files, bytes, started } => {
                let (work, progress, files, bytes, started) = (*work, progress.clone(), *files, *bytes, *started);
                self.render_working(&t, work, &progress, files, bytes, started, cx).into_any_element()
            }
            Phase::Done => self.render_done(&t, cx).into_any_element(),
            Phase::Failed { message } => {
                let message = message.clone();
                self.render_failed(&t, &message, cx).into_any_element()
            }
        };
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &AddSource, window, cx| this.add_source_dialog(window, cx)))
            .on_action(cx.listener(|this, _: &Rescan, window, cx| {
                if matches!(this.phase, Phase::Ready) {
                    this.start_scan(window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &CloseOverlay, _, cx| this.escape(cx)))
            .on_action(cx.listener(|this, _: &ShowAbout, _, cx| this.show_about(cx)))
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(t.canvas)
            .text_color(t.ink)
            .font_family(SANS)
            .child(self.render_topbar(&t, cx))
            .child(div().flex_1().min_h_0().child(body))
            .child(self.render_footer(&t, cx))
            .when(self.about, |d| d.child(self.render_about(&t, cx)))
    }
}

pub fn display_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

pub fn source_label(s: mosaico_core::DateSource) -> String {
    use mosaico_core::DateSource::*;
    match s {
        Exif => i18n::t!("app.source.exif"),
        Video => i18n::t!("app.source.video"),
        Filename => i18n::t!("app.source.filename"),
        Modified => i18n::t!("app.source.modified"),
    }
}

/// Unix seconds → local "2026-11-19 16:20".
pub fn chrono_local(secs: i64) -> String {
    chrono::DateTime::from_timestamp(secs, 0).map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default()
}
