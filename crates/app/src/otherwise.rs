//! otherwise-link: `otherwise-mosaico://` links in, other Otherwise apps in About, and the
//! people of the shared entity registry for person tags (read only: mosaico never writes it).
use mosaico_core::people::Tags;
use otherwise_link::apps::{self, AppSpec, Installed, Locations};
use otherwise_link::registry::{Error, Registry};
use otherwise_link::{Kind, Link, Root};
use std::path::PathBuf;
use std::sync::Mutex;

pub const APP_ID: &str = "mosaico";
#[cfg(target_os = "linux")]
const BUNDLE_ID: &str = "io.github.otherwisetech.mosaico";
/// People offered for tagging, at most.
const PEOPLE_LIMIT: usize = 2000;

/// What an incoming link asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Focus,
    /// Scan this folder (as the only source).
    Open(PathBuf),
    /// Show the photos tagged with this registry person.
    Person(String),
}

/// `open?path=…`, `entity/<uuid>`; anything else of ours focuses.
pub fn action(link: &Link) -> Option<Action> {
    if link.app != APP_ID {
        return None;
    }
    Some(match (link.at(0), link.at(1)) {
        (Some("open"), _) => link.get("path").map(PathBuf::from).filter(|p| p.is_absolute()).map(Action::Open).unwrap_or(Action::Focus),
        (Some("entity"), Some(id)) if uuid::Uuid::parse_str(id).is_ok() => Action::Person(id.to_ascii_lowercase()),
        _ => Action::Focus,
    })
}

#[cfg(test)]
fn open_link(path: &std::path::Path) -> Link {
    Link::new(APP_ID).segment("open").param("path", path.to_string_lossy())
}

static INBOUND: Mutex<Vec<Action>> = Mutex::new(Vec::new());

pub fn push(a: Action) {
    INBOUND.lock().unwrap_or_else(|e| e.into_inner()).push(a);
}

pub fn take() -> Vec<Action> {
    std::mem::take(&mut *INBOUND.lock().unwrap_or_else(|e| e.into_inner()))
}

/// GPUI's open-URLs callback: our links, and folders dropped on the Dock icon.
pub fn push_urls(urls: Vec<String>) {
    for u in urls {
        if let Some(a) = arg_action(&u) {
            push(a);
        }
    }
}

/// A command-line argument: one of our links (Linux `%U`, Windows URL handler), a `file://`
/// URL or a folder.
pub fn arg_action(arg: &str) -> Option<Action> {
    if let Ok(l) = Link::parse(arg) {
        return action(&l);
    }
    match arg.strip_prefix("file://") {
        Some(rest) => otherwise_link::link::decode(rest.strip_prefix("localhost").unwrap_or(rest), false).ok().map(|p| Action::Open(PathBuf::from(p))),
        None if arg.contains("://") => None,
        None => Some(Action::Open(PathBuf::from(arg))),
    }
}

/// Windows and Linux learn about our scheme at run time (macOS from Info.plist).
pub fn register_scheme() {
    std::thread::spawn(|| {
        let scheme = otherwise_link::link::app_scheme(APP_ID);
        #[cfg(windows)]
        if let Ok(exe) = std::env::current_exe() {
            use std::os::windows::process::CommandExt;
            // reg.exe is a console program: without this a window flashes on every start
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            for args in otherwise_link::register::windows_reg_commands(&scheme, &exe, crate::APP_NAME) {
                let _ = std::process::Command::new("reg").args(&args).creation_flags(CREATE_NO_WINDOW).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
            }
        }
        #[cfg(target_os = "linux")]
        {
            let apps = otherwise_link::paths::home().join(".local/share/applications");
            for file in ["mosaico.desktop".to_string(), format!("{BUNDLE_ID}.desktop")] { // i18n-ok: file names
                if apps.join(&file).is_file() {
                    let _ = otherwise_link::register::register_linux(&file, &[&scheme]);
                    break;
                }
            }
        }
        let _ = scheme;
    });
}

/// Other Otherwise apps installed on this computer with a window to open (not CLI-only), by name.
pub fn other_apps(loc: &Locations) -> Vec<(AppSpec, Installed)> {
    let mut found: Vec<_> = apps::discover(loc).into_iter().filter(|(s, i)| s.id != APP_ID && (i.path.is_some() || i.executable.is_some())).collect();
    found.sort_by_key(|(s, _)| s.name.to_lowercase());
    found
}

/// Starts an installed app: the bundle on macOS, its executable elsewhere.
pub fn launch(app: &Installed) -> std::io::Result<()> {
    if cfg!(target_os = "macos")
        && let Some(p) = &app.path
    {
        return std::process::Command::new("open").arg(p).status().map(|_| ());
    }
    let exe = app.executable.as_ref().ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?;
    std::process::Command::new(exe).spawn().map(|_| ())
}

/// The registry's people, by name, or None while the user hasn't turned the registry on.
pub fn registry_people(root: &Root) -> Option<Vec<(String, String)>> {
    let reg = match Registry::open_shared(root) {
        Ok(r) => r,
        Err(Error::Disabled) => return None,
        Err(e) => {
            eprintln!("mosaico: {e}"); // i18n-ok: developer diagnostic
            return None;
        }
    };
    let mut people: Vec<(String, String)> = reg.list(Some(Kind::Person), PEOPLE_LIMIT, 0).unwrap_or_default().into_iter().map(|e| (e.id.to_string(), e.name)).collect();
    people.sort_by_key(|(_, n)| n.to_lowercase());
    Some(people)
}

/// Brings tags up to date with the registry: merged people point to the survivor, renamed
/// people get their new name. Deleted people keep their tags under the last known name.
/// Returns whether anything changed.
pub fn follow_registry(root: &Root, tags: &mut Tags) -> bool {
    let Ok(reg) = Registry::open_shared(root) else { return false };
    let mut changed = false;
    for (id, _) in tags.people() {
        let Ok(uuid) = uuid::Uuid::parse_str(&id) else { continue };
        let Ok(Some(now)) = reg.resolve(uuid) else { continue };
        let name = reg.get(now).ok().flatten().map(|e| e.name);
        changed |= tags.repoint(&id, &now.to_string(), name.as_deref());
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use otherwise_link::Entity;
    use otherwise_link::settings::HubSettings;

    #[test]
    fn links() {
        let p = if cfg!(windows) { PathBuf::from(r"D:\Fotos\Viaje ñ") } else { PathBuf::from("/Volumes/SD/DCIM ñ") };
        assert_eq!(action(&Link::parse(&open_link(&p).to_url()).unwrap()), Some(Action::Open(p.clone())));
        assert_eq!(action(&Link::parse(&open_link(&p).app_url()).unwrap()), Some(Action::Open(p)));
        let a = |s: &str| action(&Link::parse(s).unwrap());
        assert_eq!(a("otherwise://mosaico/entity/6F0C2C1E-0000-4000-8000-000000000001"), Some(Action::Person("6f0c2c1e-0000-4000-8000-000000000001".into())));
        assert_eq!(a("otherwise://mosaico/entity/nope"), Some(Action::Focus));
        assert_eq!(a("otherwise://mosaico/open?path=rel"), Some(Action::Focus));
        assert_eq!(a("otherwise-mosaico://"), Some(Action::Focus));
        assert_eq!(a("otherwise://rama"), None);
        assert_eq!(arg_action("/tmp/x"), Some(Action::Open(PathBuf::from("/tmp/x"))));
        assert_eq!(arg_action("file:///tmp/a%20b/"), Some(Action::Open(PathBuf::from("/tmp/a b/"))));
        assert_eq!(arg_action("https://example.com"), None);
    }

    #[test]
    fn people_come_from_the_registry_only_when_it_is_on() {
        let dir = tempfile::tempdir().unwrap();
        let root = Root::at(dir.path());
        assert_eq!(registry_people(&root), None);
        assert!(!root.entities_db().exists(), "nothing created while off");
        HubSettings::update(&root, |s| s.entities = true).unwrap();
        let mut reg = Registry::open_shared(&root).unwrap();
        let ana = reg.upsert("genealogy", Entity::new(Kind::Person, "Ana").with_ref("genealogy", "I1")).unwrap().id;
        let ana2 = reg.upsert("genealogy", Entity::new(Kind::Person, "Ana Pérez").with_ref("genealogy", "I2")).unwrap().id;
        reg.upsert("vigia", Entity::new(Kind::Company, "ACME")).unwrap();
        let people = registry_people(&root).unwrap();
        assert_eq!(people.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>(), ["Ana", "Ana Pérez"]);

        let mut tags = Tags::default();
        tags.add(std::path::Path::new("/p/a.jpg"), &ana.to_string(), "Ana");
        reg.merge("otherwise-apps", ana2, ana).unwrap();
        assert!(follow_registry(&root, &mut tags));
        assert_eq!(tags.direct(std::path::Path::new("/p/a.jpg")), [ana2.to_string()]);
        assert_eq!(tags.name(&ana2.to_string()), Some("Ana Pérez"));
        assert!(!follow_registry(&root, &mut tags));
    }

    #[test]
    fn about_lists_other_installed_apps_only() {
        let dir = tempfile::tempdir().unwrap();
        let opt = dir.path().join("opt");
        for (folder, exe) in [("mosaico", "mosaico"), ("genealogy-rs", "genealogy-rs")] {
            std::fs::create_dir_all(opt.join(folder)).unwrap();
            std::fs::write(opt.join(folder).join(exe), "").unwrap();
        }
        let loc = Locations { os: apps::Os::Linux, mac_dirs: vec![], roots: vec![opt], path: vec![] };
        let ids: Vec<String> = other_apps(&loc).into_iter().map(|(s, _)| s.id).collect();
        assert_eq!(ids, ["genealogy"]);
    }
}
