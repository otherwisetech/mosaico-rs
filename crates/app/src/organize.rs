//! Organize: the panel (destination, folder template, copy/move), the plan preview, the
//! confirmation, progress, the result and undo. Nothing touches the disk before the user has
//! seen the preview and confirmed it in a dialog.
use crate::app::{MosaicoApp, Outcome, Phase, Work, chrono_local, display_name};
use crate::theme::{Tokens, tokens};
use crate::timeline::PANEL_W;
use crate::ui::{self, icon};
use crate::util::{group, human, month_names};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::IconName;
use gpui_component::input::Input;
use mosaico_core::apply::{self, Failure, Kept};
use mosaico_core::plan::{self, Mode, Options, SkipReason, TemplateError};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use std::time::Instant;

const PREVIEW_ROW_H: f32 = 30.;
/// Failures / kept files listed by name on the result screen.
const LIST_MAX: usize = 12;

pub enum PreviewRow {
    Folder { dir: PathBuf, count: usize, bytes: u64 },
    File(usize),
}

impl MosaicoApp {
    fn template_value(&self, cx: &App) -> String {
        self.template.read(cx).value().to_string()
    }

    fn choose_dest(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some(i18n::t!("app.org.dest_prompt").into()) });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(p) = paths.into_iter().next()
            {
                this.update(cx, |this, cx| {
                    this.dest = Some(p);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    pub(crate) fn render_panel(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let tpl = self.template_value(cx);
        let sample = self.library.as_ref().and_then(|l| l.items.last()).map(|i| (i.taken, i.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
        let (date, file) = sample.unwrap_or_else(|| (chrono::Local::now().naive_local(), "IMG_0001.JPG".into())); // i18n-ok: sample file name
        let example = plan::render_template(&tpl, date, &month_names());
        let template_ok = example.is_ok();
        let example_line = match example {
            Ok(p) => (i18n::t!("app.org.example", path = p.join(file).display()), t.ink_muted),
            Err(TemplateError::UnknownToken) => (i18n::t!("app.org.tpl_unknown"), t.danger),
            Err(TemplateError::Unbalanced) => (i18n::t!("app.org.tpl_unbalanced"), t.danger),
            Err(TemplateError::Empty) => (i18n::t!("app.org.tpl_empty"), t.danger),
        };
        let uncertain = self.library.as_ref().map(|l| l.count_by_source(mosaico_core::DateSource::Modified)).unwrap_or(0);
        let can_plan = self.dest.is_some() && template_ok;
        let label = |s: String| div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink_muted).child(s);
        let mode_opt = |id: &'static str, mode: Mode, title: String, help: String| {
            let on = self.mode == mode;
            let border = if on { t.accent } else { t.border };
            div()
                .id(id)
                .flex()
                .gap(px(10.))
                .p(px(10.))
                .rounded(px(10.))
                .border_1()
                .border_color(border)
                .bg(if on { t.accent_soft } else { t.surface })
                .cursor_pointer()
                .child(div().mt(px(2.)).flex_none().size(px(14.)).rounded_full().border_1().border_color(if on { t.accent } else { t.border_strong }).when(on, |d| d.border_4()))
                .child(div().flex().flex_col().gap(px(2.)).min_w_0().child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(title)).child(div().text_size(px(12.)).text_color(t.ink_muted).child(help)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.mode = mode;
                    cx.notify();
                }))
        };
        let include = self.include_uncertain;
        div()
            .id("panel")
            .flex_none()
            .w(px(PANEL_W))
            .h_full()
            .overflow_y_scroll()
            .border_l_1()
            .border_color(t.border)
            .bg(t.surface)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .p(px(18.))
                    .child(
                        div().flex().items_center().child(div().flex_1().text_size(px(17.)).font_weight(FontWeight::SEMIBOLD).child(i18n::t!("app.org.title"))).child(ui::icon_button(t, "panel-close", IconName::Close, true).on_click(cx.listener(|this, _, _, cx| {
                            this.panel = false;
                            cx.notify();
                        }))),
                    )
                    .child(div().text_size(px(12.)).text_color(t.ink_muted).child(i18n::t!("app.org.intro")))
                    .child(label(i18n::t!("app.org.dest")))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .px(px(10.))
                                    .py(px(8.))
                                    .rounded(px(10.))
                                    .bg(t.canvas)
                                    .text_size(px(12.))
                                    .truncate()
                                    .text_color(if self.dest.is_some() { t.ink } else { t.ink_muted })
                                    .child(self.dest.as_ref().map(|d| d.display().to_string()).unwrap_or_else(|| i18n::t!("app.org.dest_none"))),
                            )
                            .child(ui::outline_button(t, "dest").child(icon(IconName::FolderOpen, 14., t.ink)).child(i18n::t!("app.org.dest_choose")).on_click(cx.listener(|this, _, window, cx| this.choose_dest(window, cx)))),
                    )
                    .child(label(i18n::t!("app.org.template")))
                    .child(div().px(px(10.)).py(px(4.)).rounded(px(10.)).border_1().border_color(if template_ok { t.border } else { t.danger }).bg(t.canvas).text_size(px(13.)).child(Input::new(&self.template).appearance(false)))
                    .child(div().text_size(px(11.)).text_color(t.ink_muted).child(i18n::t!("app.org.template_help")))
                    .child(div().text_size(px(12.)).text_color(example_line.1).child(example_line.0))
                    .child(label(i18n::t!("app.org.mode")))
                    .child(mode_opt("mode-copy", Mode::Copy, i18n::t!("app.org.copy"), i18n::t!("app.org.copy_help")))
                    .child(mode_opt("mode-move", Mode::Move, i18n::t!("app.org.move"), i18n::t!("app.org.move_help")))
                    .when(uncertain > 0, |d| {
                        d.child(
                            div()
                                .id("include-uncertain")
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .cursor_pointer()
                                .child(div().flex().items_center().justify_center().flex_none().size(px(16.)).rounded(px(4.)).border_1().border_color(if include { t.accent } else { t.border_strong }).bg(if include { t.accent } else { t.surface }).when(include, |d| d.child(icon(IconName::Check, 12., t.on_fill()))))
                                .child(div().text_size(px(13.)).child(i18n::tn!("app.org.uncertain", uncertain, n = group(uncertain as u64))))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.include_uncertain = !this.include_uncertain;
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        ui::filled_button(t, "plan", if can_plan { t.accent } else { t.border_strong })
                            .justify_center()
                            .mt(px(6.))
                            .child(icon(IconName::Search, 15., t.on_fill()))
                            .child(i18n::t!("app.org.preview"))
                            .when(can_plan, |d| d.on_click(cx.listener(|this, _, window, cx| this.start_plan(window, cx)))),
                    )
                    .child(div().text_size(px(11.)).text_color(t.ink_muted).child(i18n::t!("app.org.safety"))),
            )
    }

    fn start_plan(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(lib), Some(dest)) = (self.library.clone(), self.dest.clone()) else { return };
        let opts = Options { dest_root: dest, template: self.template_value(cx), month_names: month_names(), mode: self.mode, include_uncertain: self.include_uncertain };
        let progress = Arc::new(plan::Progress::default());
        self.work_gen += 1;
        let gen_ = self.work_gen;
        self.phase = Phase::Planning { progress: progress.clone(), started: Instant::now() };
        let p = progress.clone();
        let task = cx.background_executor().spawn(async move { plan::build(&lib, &opts, &p) });
        cx.spawn_in(window, async move |this, cx| {
            let res = task.await;
            this.update(cx, |this, cx| {
                if this.work_gen != gen_ {
                    return;
                }
                match res {
                    Ok(plan) => {
                        let mut rows = Vec::with_capacity(plan.ops.len() + plan.folders.len());
                        for f in &plan.folders {
                            rows.push(PreviewRow::Folder { dir: f.dir.clone(), count: f.ops.len(), bytes: f.bytes });
                            rows.extend(f.ops.iter().map(|&i| PreviewRow::File(i)));
                        }
                        this.preview_rows = Rc::new(rows);
                        this.plan = Some(Arc::new(plan));
                        this.preview_scroll.scroll_to_item(0, ScrollStrategy::Top);
                        this.phase = Phase::Preview;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => this.phase = Phase::Ready,
                    Err(e) => this.phase = Phase::Failed { message: i18n::t!("app.error.other", detail = e) },
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn render_planning(&self, t: &Tokens, p: &plan::Progress, started: Instant, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(14.))
            .size_full()
            .child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child(i18n::t!("app.plan.title")))
            .child(div().text_size(px(13.)).text_color(t.ink_muted).child(i18n::t!("app.plan.units", n = group(p.units.load(Relaxed)), secs = format!("{:.1}", started.elapsed().as_secs_f32()))))
            .child(ui::outline_button(t, "plan-cancel").child(i18n::t!("common.cancel")).on_click(cx.listener(|this, _, _, cx| {
                if let Phase::Planning { progress, .. } = &this.phase {
                    progress.cancel.store(true, Relaxed);
                }
                this.work_gen += 1;
                this.phase = Phase::Ready;
                cx.notify();
            })))
    }

    pub(crate) fn render_preview(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(plan) = self.plan.clone() else { return div().into_any_element() };
        let n = plan.ops.len();
        let files_key = if plan.mode == Mode::Copy { "app.preview.files_copy" } else { "app.preview.files_move" };
        let stat = |text: String, color: Hsla, bg: Hsla| div().px(px(10.)).py(px(5.)).rounded(px(8.)).bg(bg).text_color(color).text_size(px(12.)).font_weight(FontWeight::MEDIUM).child(text);
        let mut stats = div().flex().flex_wrap().gap(px(6.)).child(stat(i18n::tn!(files_key, n, count = group(n as u64), size = human(plan.bytes())), t.accent, t.accent_soft)).child(stat(i18n::tn!("app.preview.folders", plan.folders.len(), count = group(plan.folders.len() as u64)), t.ink, t.canvas));
        let renamed = plan.renamed();
        if renamed > 0 {
            stats = stats.child(stat(i18n::tn!("app.preview.renamed", renamed, count = group(renamed as u64)), t.warn, t.warn_soft));
        }
        for (reason, key) in [(SkipReason::Identical, "app.preview.identical"), (SkipReason::DuplicateInSources, "app.preview.duplicates"), (SkipReason::AlreadyInPlace, "app.preview.in_place"), (SkipReason::UncertainDate, "app.preview.uncertain_skipped")] {
            let k = plan.skipped_for(reason);
            if k > 0 {
                stats = stats.child(stat(i18n::tn!(key, k, count = group(k as u64)), t.ink_muted, t.canvas));
            }
        }
        let rows = self.preview_rows.clone();
        let list = if n == 0 {
            div().flex_1().flex().items_center().justify_center().text_color(t.ink_muted).child(i18n::t!("app.preview.empty")).into_any_element()
        } else {
            let plan2 = plan.clone();
            uniform_list(
                "preview",
                rows.len(),
                cx.processor(move |_this, range: std::ops::Range<usize>, _window, cx| {
                    let t = tokens(cx);
                    range.map(|ix| preview_row(&t, &rows[ix], &plan2)).collect::<Vec<_>>()
                }),
            )
            .track_scroll(&self.preview_scroll)
            .flex_1()
            .into_any_element()
        };
        let apply_key = if plan.mode == Mode::Copy { "app.preview.apply_copy" } else { "app.preview.apply_move" };
        let apply_color = if plan.mode == Mode::Copy { t.accent } else { t.danger };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .gap(px(8.))
                    .px(px(20.))
                    .py(px(14.))
                    .border_b_1()
                    .border_color(t.border)
                    .bg(t.surface)
                    .child(div().flex().items_baseline().gap(px(10.)).child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child(i18n::t!("app.preview.title"))).child(div().text_size(px(12.)).text_color(t.ink_muted).child(i18n::t!("app.preview.subtitle"))))
                    .child(div().text_size(px(12.)).text_color(t.ink_muted).child(i18n::t!("app.preview.dest", path = plan.dest_root.display())))
                    .child(stats),
            )
            .child(div().flex().flex_col().flex_1().min_h_0().bg(t.surface).child(list))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.))
                    .px(px(20.))
                    .py(px(10.))
                    .border_t_1()
                    .border_color(t.border)
                    .bg(t.canvas)
                    .child(div().flex_1().text_size(px(12.)).text_color(if plan.mode == Mode::Move { t.danger } else { t.ink_muted }).child(if plan.mode == Mode::Copy { i18n::t!("app.org.copy_help") } else { i18n::t!("app.org.move_help") }))
                    .child(ui::outline_button(t, "preview-back").child(i18n::t!("app.preview.back")).on_click(cx.listener(|this, _, _, cx| {
                        this.phase = Phase::Ready;
                        cx.notify();
                    })))
                    .when(n > 0, |d| d.child(ui::filled_button(t, "apply", apply_color).child(i18n::tn!(apply_key, n, count = group(n as u64))).on_click(cx.listener(|this, _, window, cx| this.confirm_apply(window, cx))))),
            )
            .into_any_element()
    }

    fn confirm_apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(plan) = self.plan.clone() else { return };
        let n = plan.ops.len();
        let path = plan.dest_root.display().to_string();
        let (msg, detail, ok, level) = match plan.mode {
            Mode::Copy => (i18n::tn!("app.confirm.copy_title", n, count = group(n as u64), path = path), i18n::t!("app.confirm.copy_detail"), i18n::t!("app.confirm.copy_ok"), PromptLevel::Info),
            Mode::Move => (i18n::tn!("app.confirm.move_title", n, count = group(n as u64), path = path), i18n::t!("app.confirm.move_detail"), i18n::t!("app.confirm.move_ok"), PromptLevel::Warning),
        };
        let cancel = i18n::t!("common.cancel");
        let rx = window.prompt(level, &msg, Some(&detail), &[ok.as_str(), cancel.as_str()], cx);
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(0) = rx.await {
                this.update(cx, |this, cx| this.start_apply(plan, cx)).ok();
            }
        })
        .detach();
    }

    fn start_apply(&mut self, plan: Arc<plan::Plan>, cx: &mut Context<Self>) {
        let Some(jdir) = mosaico_core::dirs::journal_dir() else {
            self.phase = Phase::Failed { message: i18n::t!("app.error.no_data_dir") };
            cx.notify();
            return;
        };
        let progress = Arc::new(apply::Progress::default());
        self.phase = Phase::Working { work: Work::Apply(plan.mode), progress: progress.clone(), files: plan.ops.len() as u64, bytes: plan.bytes(), started: Instant::now() };
        let p = progress.clone();
        let plan2 = plan.clone();
        let task = cx.background_executor().spawn(async move { apply::apply(&plan2, &jdir, &p) });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |this, cx| {
                match res {
                    Ok(report) => {
                        this.outcome = Some(Outcome::Applied { report, mode: plan.mode, dest: plan.dest_root.clone() });
                        this.phase = Phase::Done;
                    }
                    Err(e) => this.phase = Phase::Failed { message: i18n::t!("app.error.journal", detail = e) },
                }
                this.panel = false;
                this.refresh_journal();
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_working(&self, t: &Tokens, work: Work, p: &apply::Progress, files: u64, bytes: u64, started: Instant, cx: &mut Context<Self>) -> impl IntoElement {
        let done = p.files.load(Relaxed).min(files);
        let title = match work {
            Work::Apply(Mode::Copy) => i18n::t!("app.work.copying"),
            Work::Apply(Mode::Move) => i18n::t!("app.work.moving"),
            Work::Undo => i18n::t!("app.work.undoing"),
        };
        let frac = if files == 0 { 1. } else { done as f32 / files as f32 };
        let cancelling = p.cancel.load(Relaxed);
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(16.))
            .size_full()
            .child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child(title))
            .child(div().w(px(460.)).child(ui::progress_bar(t, frac, t.accent)))
            .child(div().text_size(px(13.)).text_color(t.ink_muted).child(i18n::t!("app.work.progress", done = group(done), total = group(files), size = human(if work == Work::Undo { 0 } else { p.bytes.load(Relaxed).min(bytes) }), secs = format!("{:.0}", started.elapsed().as_secs_f32()))))
            .when(matches!(work, Work::Apply(_)), |d| {
                d.child(if cancelling {
                    div().text_size(px(12.)).text_color(t.ink_muted).child(i18n::t!("app.work.stopping")).into_any_element()
                } else {
                    ui::outline_button(t, "apply-cancel")
                        .child(i18n::t!("app.work.stop"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Phase::Working { progress, .. } = &this.phase {
                                progress.cancel.store(true, Relaxed);
                            }
                            cx.notify();
                        }))
                        .into_any_element()
                })
            })
            .child(div().max_w(px(460.)).text_center().text_size(px(12.)).text_color(t.ink_muted).child(i18n::t!("app.work.note")))
    }

    pub(crate) fn render_done(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let line = |ic: IconName, color: Hsla, text: String| div().flex().items_center().gap(px(8.)).text_size(px(14.)).child(icon(ic, 16., color)).child(text);
        let small = |text: String| div().pl(px(24.)).text_size(px(12.)).text_color(t.ink_muted).truncate().child(text);
        let mut body = div().flex().flex_col().gap(px(8.)).w(px(560.));
        let mut title = i18n::t!("app.done.title");
        let mut dest_open: Option<PathBuf> = None;
        let mut can_undo = false;
        match &self.outcome {
            Some(Outcome::Applied { report: r, mode, dest }) => {
                if r.cancelled {
                    title = i18n::t!("app.done.stopped");
                }
                dest_open = Some(dest.clone());
                can_undo = r.done > 0 && self.last_journal.as_ref().is_some_and(|j| j.path == r.journal);
                let key = if *mode == Mode::Copy { "app.done.copied" } else { "app.done.moved" };
                body = body.child(line(IconName::CircleCheck, t.ok, i18n::tn!(key, r.done, count = group(r.done as u64), size = human(r.bytes))));
                if r.identical > 0 {
                    body = body.child(line(IconName::Info, t.ink_muted, i18n::tn!("app.done.identical", r.identical, count = group(r.identical as u64))));
                }
                if !r.failed.is_empty() {
                    body = body.child(line(IconName::CircleX, t.danger, i18n::tn!("app.done.failed", r.failed.len(), count = group(r.failed.len() as u64))));
                    for (p, f) in r.failed.iter().take(LIST_MAX) {
                        let why = match f {
                            Failure::DestinationExists => i18n::t!("app.done.fail_exists"),
                            Failure::VerifyFailed => i18n::t!("app.done.fail_verify"),
                            Failure::Io(e) => e.clone(),
                        };
                        body = body.child(small(format!("{} — {why}", display_name(p))));
                    }
                }
                if !r.source_kept.is_empty() {
                    body = body.child(line(IconName::TriangleAlert, t.warn, i18n::tn!("app.done.source_kept", r.source_kept.len(), count = group(r.source_kept.len() as u64))));
                    for (p, e) in r.source_kept.iter().take(LIST_MAX) {
                        body = body.child(small(format!("{} — {e}", display_name(p))));
                    }
                }
            }
            Some(Outcome::Undone(u)) => {
                title = i18n::t!("app.done.undone_title");
                body = body.child(line(IconName::CircleCheck, t.ok, i18n::tn!("app.done.restored", u.restored, count = group(u.restored as u64))));
                if !u.kept.is_empty() {
                    body = body.child(line(IconName::TriangleAlert, t.warn, i18n::tn!("app.done.kept", u.kept.len(), count = group(u.kept.len() as u64))));
                    for (p, k) in u.kept.iter().take(LIST_MAX) {
                        let why = match k {
                            Kept::Changed => i18n::t!("app.done.kept_changed"),
                            Kept::Missing => i18n::t!("app.done.kept_missing"),
                            Kept::SourceMissing => i18n::t!("app.done.kept_source_missing"),
                            Kept::Io(e) => e.clone(),
                        };
                        body = body.child(small(format!("{} — {why}", p.display())));
                    }
                }
                if u.dirs_left > 0 {
                    body = body.child(line(IconName::Folder, t.ink_muted, i18n::tn!("app.done.dirs_left", u.dirs_left, count = group(u.dirs_left as u64))));
                }
            }
            None => {}
        }
        div()
            .id("done")
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(18.))
                    .py(px(56.))
                    .child(div().text_size(px(22.)).font_weight(FontWeight::SEMIBOLD).child(title))
                    .child(body)
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .children(dest_open.filter(|d| d.exists()).map(|d| ui::outline_button(t, "reveal").child(icon(IconName::FolderOpen, 15., t.ink)).child(i18n::t!("app.done.open_dest")).on_click(move |_, _, cx| cx.open_with_system(&d))))
                            .when(can_undo, |d| d.child(ui::outline_button(t, "undo").child(icon(IconName::Undo2, 15., t.ink)).child(i18n::t!("app.done.undo")).on_click(cx.listener(|this, _, window, cx| this.confirm_undo(window, cx)))))
                            .child(ui::filled_button(t, "back-timeline", t.accent).child(i18n::t!("app.done.back")).on_click(cx.listener(|this, _, window, cx| {
                                // files moved (or copies removed): read the sources again
                                this.outcome = None;
                                this.start_scan(window, cx);
                            }))),
                    ),
            )
    }

    pub(crate) fn confirm_undo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_journal();
        let Some(j) = self.last_journal.clone() else { return };
        let when = chrono_local(j.created);
        let msg = i18n::t!("app.undo.confirm_title", when = when);
        let detail = match j.mode {
            Mode::Copy => i18n::tn!("app.undo.confirm_copy", j.placed, count = group(j.placed as u64)),
            Mode::Move => i18n::tn!("app.undo.confirm_move", j.placed, count = group(j.placed as u64)),
        };
        let (ok, cancel) = (i18n::t!("app.undo.button"), i18n::t!("common.cancel"));
        let rx = window.prompt(PromptLevel::Warning, &msg, Some(&detail), &[ok.as_str(), cancel.as_str()], cx);
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(0) = rx.await {
                this.update(cx, |this, cx| this.start_undo(j.path.clone(), j.placed as u64, cx)).ok();
            }
        })
        .detach();
    }

    fn start_undo(&mut self, path: PathBuf, files: u64, cx: &mut Context<Self>) {
        let progress = Arc::new(apply::Progress::default());
        self.phase = Phase::Working { work: Work::Undo, progress: progress.clone(), files, bytes: 0, started: Instant::now() };
        let p = progress.clone();
        let task = cx.background_executor().spawn(async move { apply::undo(&path, &p) });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |this, cx| {
                match res {
                    Ok(u) => {
                        this.outcome = Some(Outcome::Undone(u));
                        this.phase = Phase::Done;
                    }
                    Err(e) => this.phase = Phase::Failed { message: i18n::t!("app.error.other", detail = e) },
                }
                this.refresh_journal();
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}

fn preview_row(t: &Tokens, row: &PreviewRow, plan: &plan::Plan) -> AnyElement {
    match row {
        PreviewRow::Folder { dir, count, bytes } => div()
            .flex()
            .items_center()
            .gap(px(8.))
            .h(px(PREVIEW_ROW_H))
            .px(px(20.))
            .bg(t.canvas)
            .child(icon(IconName::Folder, 14., t.accent))
            .child(div().flex_1().min_w_0().truncate().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(dir.display().to_string()))
            .child(div().flex_none().text_size(px(11.)).text_color(t.ink_muted).child(i18n::tn!("app.grid.month_count", *count, count = group(*count as u64), size = human(*bytes))))
            .into_any_element(),
        PreviewRow::File(i) => {
            let op = &plan.ops[*i];
            let name = op.dst.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let from = op.src.display().to_string();
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(PREVIEW_ROW_H))
                .pl(px(42.))
                .pr(px(20.))
                .child(div().flex_none().max_w(px(320.)).truncate().text_size(px(13.)).child(name))
                .when(op.renamed, |d| d.child(ui::badge(i18n::t!("app.preview.renamed_badge"), t.warn, t.warn_soft)))
                .when(op.uncertain, |d| d.child(ui::badge(i18n::t!("app.preview.uncertain_badge"), t.warn, t.warn_soft)))
                .child(icon(IconName::ArrowLeft, 12., t.ink_muted))
                .child(div().flex_1().min_w_0().truncate().text_size(px(11.)).text_color(t.ink_muted).child(from))
                .child(div().flex_none().text_size(px(11.)).text_color(t.ink_muted).child(human(op.size)))
                .into_any_element()
        }
    }
}
