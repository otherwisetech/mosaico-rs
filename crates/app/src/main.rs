// No console window behind the app on Windows release builds.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod organize;
mod otherwise;
mod people_view;
mod settings;
mod star;
mod storage;
mod vcity;
mod theme;
mod thumbs;
mod timeline;
mod ui;
mod util;

use gpui::*;
use gpui_component::Root;
use gpui_component_assets::Assets;
use std::borrow::Cow;
use std::path::PathBuf;

actions!(mosaico, [Quit, OpenWebsite, OpenVcity, RevealData]);

pub(crate) const APP_NAME: &str = "mosaico"; // i18n-ok: product name
pub(crate) const COMPANY: &str = otherwise_ui::COMPANY;
pub(crate) const COMPANY_URL: &str = otherwise_ui::COMPANY_URL;
pub(crate) const REPO_URL: &str = "https://github.com/otherwisetech/mosaico-rs";
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The native menu bar holds plain strings, so it is rebuilt on every language switch.
pub(crate) fn set_app_menus(cx: &mut App) {
    let items = vec![
        MenuItem::action(i18n::t!("app.about.menu", app = APP_NAME), app::ShowAbout),
        MenuItem::action(i18n::t!("app.credit.menu", company = COMPANY), OpenWebsite),
        MenuItem::separator(),
        MenuItem::action(i18n::t!("app.file.reveal_data"), RevealData),
        MenuItem::separator(),
        MenuItem::action(i18n::t!("app.menu.quit"), Quit),
    ];
    let help = vec![MenuItem::action(vcity::menu_label(), OpenVcity)];
    cx.set_menus(vec![Menu { name: APP_NAME.into(), items, disabled: false }, Menu { name: i18n::t!("app.menu.help").into(), items: help, disabled: false }]);
}

/// gpui-component's icon set plus our own `icons/*.svg`.
struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match path {
            LOGO_SVG => Ok(Some(Cow::Borrowed(include_bytes!("../assets/icons/mosaico-logo.svg")))),
            _ => Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Assets.list(path)
    }
}

pub(crate) const LOGO_SVG: &str = "icons/mosaico-logo.svg";
pub(crate) static ICON_PNG: &[u8] = include_bytes!("../assets/logo/mosaico-icon-512.png");

/// Sets the Dock icon at runtime: covers `cargo run` (no bundle) and stale Dock icon caches.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // objc 0.2 macros check a `cargo-clippy` feature
fn set_dock_icon() {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    static PNG: &[u8] = include_bytes!("../assets/logo/mosaico-icon-1024.png");
    // SAFETY: plain AppKit calls on the main thread with a valid byte buffer that NSData copies.
    unsafe {
        let data: *mut Object = msg_send![class!(NSData), dataWithBytes: PNG.as_ptr() length: PNG.len()];
        let image: *mut Object = msg_send![class!(NSImage), alloc];
        let image: *mut Object = msg_send![image, initWithData: data];
        if image.is_null() {
            return;
        }
        let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, setApplicationIconImage: image];
        let _: () = msg_send![image, release];
    }
}

fn main() {
    storage::init();
    i18n::set_locale(settings::initial_language(&settings::Settings::load()));
    if matches!(std::env::args().nth(1).as_deref(), Some("-h" | "--help")) {
        println!("{}", i18n::t!("app.cli.usage"));
        return;
    }
    let mut initial: Vec<PathBuf> = Vec::new();
    for a in std::env::args().skip(1).filter_map(|a| otherwise::arg_action(&a)) {
        match a {
            otherwise::Action::Open(p) => initial.push(p),
            other => otherwise::push(other),
        }
    }
    otherwise::register_scheme();
    let application = gpui_platform::application().with_assets(AppAssets);
    application.on_open_urls(otherwise::push_urls);
    application.run(move |cx| {
        #[cfg(target_os = "macos")]
        set_dock_icon();
        gpui_component::init(cx);
        otherwise_ui::init(otherwise_ui::accents::MOSAICO, cx);
        app::sync_theme(None, cx);
        app::bind_keys(cx);
        cx.bind_keys([KeyBinding::new("secondary-q", Quit, None)]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &OpenWebsite, cx| cx.open_url(COMPANY_URL));
        cx.on_action(|_: &OpenVcity, cx| vcity::open(cx));
        cx.on_action(|_: &RevealData, cx| storage::reveal_data_dir(cx));
        gpui_component::set_locale(i18n::locale());
        set_app_menus(cx);
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1440.), px(900.)), cx)),
            titlebar: Some(TitlebarOptions { title: Some(APP_NAME.into()), ..Default::default() }),
            ..Default::default()
        };
        let initial = initial.clone();
        cx.spawn(async move |cx| {
            cx.open_window(window_options, |window, cx| {
                let view = cx.new(|cx| app::MosaicoApp::new(initial, window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("open window");
        })
        .detach();
    });
}
