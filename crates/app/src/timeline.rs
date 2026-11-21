//! The timeline: years and months on the left, a virtualised grid of thumbnails (rows of one
//! month each; the first row of a month carries its name) and, optionally, the organize panel.
use crate::app::{GridRow, MosaicoApp};
use crate::theme::{Tokens, tokens};
use crate::thumbs::Thumb;
use crate::ui::{self, icon};
use crate::util::{group, human, month_name};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::IconName;
use mosaico_core::Kind;
use std::rc::Rc;

pub const TILE: f32 = 132.;
const GAP: f32 = 6.;
const GUTTER: f32 = 150.;
const SIDE_W: f32 = 236.;
pub const PANEL_W: f32 = 380.;

impl MosaicoApp {
    /// Lay the months out in rows of `cols` tiles (cached until the width, filter or library change).
    fn ensure_rows(&mut self, cols: usize) {
        let key = (cols, self.only_uncertain, self.lib_gen);
        if key == self.rows_key {
            return;
        }
        let mut rows = Vec::new();
        let mut month_row = Vec::with_capacity(self.months.len());
        for (mi, m) in self.months.iter().enumerate() {
            month_row.push(rows.len());
            let mut start = 0;
            while start < m.items.len() {
                let end = (start + cols).min(m.items.len());
                rows.push(GridRow { month: mi, first: start == 0, start, end });
                start = end;
            }
        }
        self.rows = Rc::new(rows);
        self.month_row = month_row;
        self.rows_key = key;
    }

    pub(crate) fn render_timeline(&mut self, t: &Tokens, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = window.viewport_size().width / px(1.);
        let avail = width - SIDE_W - if self.panel { PANEL_W } else { 0. } - GUTTER - 36.;
        let cols = (((avail + GAP) / (TILE + GAP)).floor() as usize).max(1);
        self.ensure_rows(cols);
        let center = if self.rows.is_empty() {
            let msg = if self.person.is_some() {
                i18n::t!("app.people.empty")
            } else if self.only_uncertain { i18n::t!("app.grid.empty_uncertain") } else { i18n::t!("app.grid.empty") };
            div().size_full().flex().items_center().justify_center().text_color(t.ink_muted).text_size(px(14.)).child(msg).into_any_element()
        } else {
            let n = self.rows.len();
            uniform_list(
                "grid",
                n,
                cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                    let t = tokens(cx);
                    range.map(|ix| this.grid_row(&t, ix, cx)).collect::<Vec<_>>()
                }),
            )
            .track_scroll(&self.grid_scroll)
            .size_full()
            .into_any_element()
        };
        div()
            .flex()
            .size_full()
            .min_h_0()
            .child(self.render_sidebar(t, cx))
            .child(div().flex_1().min_w_0().h_full().flex().flex_col().bg(t.surface).child(div().flex_1().min_h_0().child(center)).children(self.render_people_bar(t, cx)))
            .when(self.panel, |d| d.child(self.render_panel(t, cx)))
    }

    fn render_sidebar(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let people = self.render_people_list(t, cx);
        let mut list = div().flex().flex_col().gap(px(1.)).p(px(8.));
        let mut mi = 0;
        let hover = t.canvas;
        for y in self.years.iter() {
            let year = y.year;
            let collapsed = self.collapsed_years.contains(&year);
            list = list.child(
                div()
                    .id(("year", year as u32 as usize))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(8.))
                    .py(px(6.))
                    .rounded(px(8.))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .child(icon(if collapsed { IconName::ChevronRight } else { IconName::ChevronDown }, 13., t.ink_muted))
                    .child(div().flex_1().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(year.to_string()))
                    .child(div().text_size(px(11.)).text_color(t.ink_muted).child(format!("{} · {}", group(y.count as u64), human(y.bytes))))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.collapsed_years.remove(&year) {
                            this.collapsed_years.insert(year);
                        }
                        cx.notify();
                    })),
            );
            for m in &y.months {
                let index = mi;
                mi += 1;
                if collapsed {
                    continue;
                }
                list = list.child(
                    div()
                        .id(("month", index))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .pl(px(27.))
                        .pr(px(8.))
                        .py(px(4.))
                        .rounded(px(8.))
                        .cursor_pointer()
                        .hover(move |s| s.bg(hover))
                        .child(div().flex_1().text_size(px(13.)).child(month_name(m.month)))
                        .when(m.uncertain > 0, |d| d.child(div().size(px(6.)).rounded_full().bg(t.warn)))
                        .child(div().text_size(px(11.)).text_color(t.ink_muted).child(group(m.items.len() as u64)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(&row) = this.month_row.get(index) {
                                this.grid_scroll.scroll_to_item(row, ScrollStrategy::Top);
                                cx.notify();
                            }
                        })),
                );
            }
        }
        div().id("sidebar").flex_none().w(px(SIDE_W)).h_full().overflow_y_scroll().border_r_1().border_color(t.border).bg(t.canvas).children(people).child(list)
    }

    fn grid_row(&mut self, t: &Tokens, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let row = self.rows[ix].clone();
        let months = self.months.clone();
        let Some(lib) = self.library.clone() else { return div().into_any_element() };
        let m = &months[row.month];
        let gutter = div().flex_none().w(px(GUTTER)).pr(px(12.)).when(row.first, |d| {
            d.flex()
                .flex_col()
                .gap(px(2.))
                .pt(px(4.))
                .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(month_name(m.month)))
                .child(div().text_size(px(13.)).text_color(t.ink_muted).child(m.year.to_string()))
                .child(div().text_size(px(11.)).text_color(t.ink_muted).child(i18n::tn!("app.grid.month_count", m.items.len(), count = group(m.items.len() as u64), size = human(m.bytes))))
                .when(m.uncertain > 0, |d| d.child(ui::badge(i18n::tn!("app.grid.month_uncertain", m.uncertain), t.warn, t.warn_soft).mt(px(4.))))
        });
        let mut tiles = div().flex().gap(px(GAP));
        for k in row.start..row.end {
            let item_ix = m.items[k];
            let it = &lib.items[item_ix];
            let thumb = self.thumbs.get(item_ix, it);
            tiles = tiles.child(self.tile(t, item_ix, it, thumb, cx));
        }
        div()
            .flex()
            .h(px(TILE + GAP))
            .px(px(14.))
            .pt(px(GAP))
            .when(row.first && row.month > 0, |d| d.border_t_1().border_color(t.border))
            .child(gutter)
            .child(tiles)
            .into_any_element()
    }

    fn tile(&self, t: &Tokens, ix: usize, it: &mosaico_core::Item, thumb: Thumb, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.selected == Some(ix);
        let content: AnyElement = match thumb {
            Thumb::Ready(p) => img(p).size_full().object_fit(ObjectFit::Cover).image_cache(&self.images).into_any_element(),
            Thumb::Pending => div().into_any_element(),
            Thumb::Placeholder => div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(6.))
                .child(icon(if it.format.kind() == Kind::Video { IconName::Play } else { IconName::File }, 22., t.ink_muted))
                .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink_muted).child(it.format.label()))
                .into_any_element(),
        };
        let path = it.path.clone();
        div()
            .id(("tile", ix))
            .relative()
            .flex_none()
            .size(px(TILE))
            .rounded(px(8.))
            .overflow_hidden()
            .bg(t.tile)
            .cursor_pointer()
            .child(content)
            .when(it.format.kind() == Kind::Video, |d| d.child(div().absolute().left(px(6.)).bottom(px(6.)).p(px(3.)).rounded(px(6.)).bg(gpui::black().opacity(0.55)).child(icon(IconName::Play, 11., gpui::white()))))
            .when(it.uncertain(), |d| d.child(div().absolute().right(px(6.)).top(px(6.)).size(px(10.)).rounded_full().border_2().border_color(gpui::white()).bg(t.warn)))
            .when(selected, |d| d.child(div().absolute().inset_0().rounded(px(8.)).border_3().border_color(t.accent)))
            .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                if ev.click_count() >= 2 {
                    cx.open_with_system(&path);
                }
                this.selected = Some(ix);
                cx.notify();
            }))
    }
}
