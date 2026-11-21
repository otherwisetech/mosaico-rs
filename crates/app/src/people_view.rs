//! Person tags in the timeline: the People list in the sidebar (filters the grid), the bar under
//! the grid for the selected photo (its people, tag the photo or its folder), the link pump and
//! About's other Otherwise apps.
use crate::app::{MosaicoApp, Phase};
use crate::otherwise::{self, Action};
use crate::theme::Tokens;
use crate::ui::{self, icon};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_component::{Disableable as _, IconName, Sizable as _};
use otherwise_link::Root;
use std::path::{Path, PathBuf};
use std::time::Duration;

impl MosaicoApp {
    /// Registry people and merges, off the UI thread (start, window activation).
    pub fn refresh_people(&mut self, cx: &mut Context<Self>) {
        let mut tags = self.tags.clone();
        let task = cx.background_executor().spawn(async move {
            let root = Root::system();
            let changed = otherwise::follow_registry(&root, &mut tags);
            (otherwise::registry_people(&root), changed.then_some(tags))
        });
        cx.spawn(async move |this, cx| {
            let (people, tags) = task.await;
            this.update(cx, |this, cx| {
                this.people = people;
                if let Some(t) = tags {
                    this.tags = t;
                    this.save_tags();
                    this.apply_person_filter();
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn save_tags(&self) {
        // a people.json we couldn't read is never replaced
        if let (true, Some(f)) = (self.tags_ok, &self.tags_file)
            && let Err(e) = self.tags.save(f)
        {
            eprintln!("mosaico: {e}"); // i18n-ok: developer diagnostic
        }
    }

    pub fn tag(&mut self, path: PathBuf, id: String, name: String, cx: &mut Context<Self>) {
        if self.tags_ok && self.tags.add(&path, &id, &name) {
            self.save_tags();
            self.apply_person_filter();
            cx.notify();
        }
    }

    pub fn untag(&mut self, path: PathBuf, id: String, cx: &mut Context<Self>) {
        if self.tags_ok && self.tags.remove(&path, &id) {
            self.save_tags();
            self.apply_person_filter();
            cx.notify();
        }
    }

    pub fn set_person_filter(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        self.person = id;
        self.apply_person_filter();
        cx.notify();
    }

    /// Rebuild the grid after the person filter or the tags changed.
    pub fn apply_person_filter(&mut self) {
        if self.person.is_some() {
            self.lib_gen += 1;
            self.rebuild_timeline();
        }
    }

    /// Drains links from the OS every 300 ms.
    pub fn watch_links(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(300)).await;
                let actions = otherwise::take();
                if actions.is_empty() {
                    continue;
                }
                let alive = this.update_in(cx, |this, window, cx| this.handle_links(actions, window, cx)).is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();
    }

    fn handle_links(&mut self, actions: Vec<Action>, window: &mut Window, cx: &mut Context<Self>) {
        window.activate_window();
        cx.activate(true);
        let folders: Vec<PathBuf> = actions.iter().filter_map(|a| if let Action::Open(p) = a { Some(p.clone()) } else { None }).filter(|p| p.is_dir()).collect();
        let busy = matches!(self.phase, Phase::Working { .. } | Phase::Planning { .. } | Phase::Preview);
        if !folders.is_empty() && !busy {
            self.sources = folders;
            self.person = None;
            self.start_scan(window, cx);
        }
        if let Some(id) = actions.iter().rev().find_map(|a| if let Action::Person(id) = a { Some(id.clone()) } else { None }) {
            if self.library.is_some() && !busy {
                self.set_person_filter(Some(id), cx);
            } else {
                // after the next scan
                self.person = Some(id);
            }
        }
        cx.notify();
    }

    /// "People" at the top of the sidebar, once someone is tagged.
    pub(crate) fn render_people_list(&self, t: &Tokens, cx: &mut Context<Self>) -> Option<Div> {
        let people = self.tags.people();
        if people.is_empty() && self.person.is_none() {
            return None;
        }
        let hover = t.canvas;
        let row = |id: &'static str, ix: usize, label: String, on: bool| {
            div()
                .id((id, ix))
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(8.))
                .py(px(4.))
                .rounded(px(8.))
                .cursor_pointer()
                .text_size(px(13.))
                .when(on, |d| d.bg(t.accent_soft).text_color(t.accent).font_weight(FontWeight::SEMIBOLD))
                .when(!on, |d| d.hover(move |s| s.bg(hover)))
                .child(icon(IconName::CircleUser, 13., if on { t.accent } else { t.ink_muted }))
                .child(div().flex_1().min_w_0().truncate().child(label))
        };
        let mut list = div().flex().flex_col().gap(px(1.)).px(px(8.)).pt(px(8.)).child(div().px(px(8.)).pb(px(4.)).child(ui::section_title(t, i18n::t!("app.people.title"))));
        list = list.child(row("person-all", 0, i18n::t!("app.people.all"), self.person.is_none()).on_click(cx.listener(|this, _, _, cx| this.set_person_filter(None, cx))));
        for (i, (id, name)) in people.into_iter().enumerate() {
            let on = self.person.as_deref() == Some(id.as_str());
            list = list.child(row("person", i, name, on).on_click(cx.listener(move |this, _, _, cx| this.set_person_filter(Some(id.clone()), cx))));
        }
        Some(list.child(div().mx(px(8.)).mt(px(6.)).h(px(1.)).bg(t.border)))
    }

    /// Under the grid while a photo is selected: who is in it, and buttons to tag it or its folder.
    pub(crate) fn render_people_bar(&self, t: &Tokens, cx: &mut Context<Self>) -> Option<Div> {
        let lib = self.library.as_ref()?;
        let item = lib.items.get(self.selected?)?;
        let path = item.path.clone();
        let folder = path.parent().map(Path::to_path_buf);
        let chips = self.tags.of_item(&path).into_iter().enumerate().map(|(i, (id, on))| {
            let name = self.tags.name(&id).unwrap_or(&id).to_string();
            let label = if on == path { name } else { i18n::t!("app.people.in_folder", name = name) };
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .pl(px(8.))
                .pr(px(2.))
                .h(px(24.))
                .rounded(px(12.))
                .bg(t.accent_soft)
                .text_color(t.accent)
                .text_size(px(12.))
                .child(label)
                .child(ui::icon_button(t, ("untag", i), IconName::Close, true).size(px(20.)).on_click(cx.listener(move |this, _, _, cx| this.untag(on.clone(), id.clone(), cx))))
        });
        let menu_button = |id: &'static str, label: String, target: Option<PathBuf>| -> AnyElement {
            let Some(target) = target else { return div().into_any_element() };
            let Some(people) = self.people.clone() else {
                return Button::new(id).ghost().small().label(label).disabled(true).tooltip(i18n::t!("app.people.registry_off")).into_any_element();
            };
            let weak = cx.entity().downgrade();
            let tagged: Vec<String> = self.tags.direct(&target).to_vec();
            Button::new(id)
                .outline()
                .small()
                .icon(IconName::Plus)
                .label(label)
                .dropdown_menu(move |mut menu, _, _| {
                    if people.is_empty() {
                        return menu.label(i18n::t!("app.people.none"));
                    }
                    for (pid, name) in &people {
                        let (w, pid, name, target) = (weak.clone(), pid.clone(), name.clone(), target.clone());
                        let on = tagged.contains(&pid);
                        menu = menu.item(PopupMenuItem::new(name.clone()).checked(on).on_click(move |_, _, cx| {
                            let (pid, name, target) = (pid.clone(), name.clone(), target.clone());
                            w.update(cx, |this, cx| if on { this.untag(target, pid, cx) } else { this.tag(target, pid, name, cx) }).ok();
                        }));
                    }
                    menu
                })
                .into_any_element()
        };
        Some(
            div()
                .flex()
                .flex_none()
                .flex_wrap()
                .items_center()
                .gap(px(6.))
                .px(px(14.))
                .py(px(8.))
                .border_t_1()
                .border_color(t.border)
                .bg(t.surface)
                .child(icon(IconName::CircleUser, 14., t.ink_muted))
                .child(div().max_w(px(260.)).truncate().text_size(px(12.)).text_color(t.ink_muted).child(crate::app::display_name(&path)))
                .children(chips)
                .child(menu_button("tag-photo", i18n::t!("app.people.tag_photo"), Some(path.clone())))
                .child(menu_button("tag-folder", i18n::t!("app.people.tag_folder"), folder)),
        )
    }

    pub fn find_other_apps(&mut self, cx: &mut Context<Self>) {
        let task = cx.background_executor().spawn(async { otherwise::other_apps(&otherwise_link::apps::Locations::system()) });
        cx.spawn(async move |this, cx| {
            let found = task.await;
            this.update(cx, |this, cx| {
                this.other_apps = found;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// "More Otherwise apps": the ones installed here, each opening on click.
    pub(crate) fn render_other_apps(&self, t: &Tokens) -> Option<Div> {
        if self.other_apps.is_empty() {
            return None;
        }
        let rows = self.other_apps.iter().enumerate().map(|(i, (spec, installed))| {
            let installed = installed.clone();
            let hover = t.canvas;
            div()
                .id(("about-app", i))
                .flex()
                .items_center()
                .justify_between()
                .gap(px(10.))
                .px(px(10.))
                .py(px(5.))
                .rounded(px(8.))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .child(div().text_size(px(13.)).child(spec.name.clone()))
                .child(div().text_size(px(11.)).text_color(t.ink_muted).child(installed.version.clone().unwrap_or_default()))
                .on_click(move |_, _, _| {
                    let app = installed.clone();
                    std::thread::spawn(move || otherwise::launch(&app));
                })
        });
        Some(div().w_full().flex().flex_col().gap(px(4.)).child(ui::section_title(t, i18n::t!("app.about.more_apps"))).children(rows))
    }
}
