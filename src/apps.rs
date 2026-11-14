//! Finds installed apps and the files each one has left around the system.

use crate::rules::{Finding, Group};
use crate::Node;
use rayon::prelude::*;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Installed {
    pub name: String,
    /// The app's internal name when it differs from the file name ("Code" for Visual Studio Code).
    pub alt: Option<String>,
    pub id: String, // bundle identifier, e.g. com.tinyapp.TablePlus
    pub icon: Option<String>,
    pub path: PathBuf,
}

// Where apps keep things: (folder, what a match there is, may match by app name).
// Matching by name is only allowed where apps conventionally use their own name as the
// folder name; everywhere else only the bundle identifier counts.
const SPOTS: &[(&str, &str, bool)] = &[
    ("~/Library/Application Support", "Its data and settings.", true),
    ("~/Library/Caches", "Its cache.", true),
    ("~/Library/Logs", "Its logs.", true),
    ("~/Library/Preferences", "Its preferences.", false),
    ("~/Library/Preferences/ByHost", "Its preferences.", false),
    ("~/Library/Containers", "Its sandbox: everything the app stored.", false),
    ("~/Library/Group Containers", "Data it shares with its helper apps.", false),
    ("~/Library/Application Scripts", "Its script folder.", false),
    ("~/Library/Saved Application State", "Its saved window state.", false),
    ("~/Library/HTTPStorages", "Its web storage.", false),
    ("~/Library/WebKit", "Its web storage.", false),
    ("~/Library/Cookies", "Its cookies.", false),
    ("~/Library/LaunchAgents", "A background helper it installed.", false),
    ("/Library/LaunchAgents", "A background helper it installed (needs admin).", false),
    ("/Library/LaunchDaemons", "A system service it installed (needs admin).", false),
    ("/Library/Application Support", "Its shared data (needs admin).", true),
    ("/Library/Preferences", "Its system-wide preferences (needs admin).", false),
    ("/Library/Caches", "Its system-wide cache (needs admin).", true),
];

fn plist_value(app: &Path, key: &str) -> Option<String> {
    let out = Command::new("plutil")
        .args(["-extract", key, "raw", "-o", "-"])
        .arg(app.join("Contents/Info.plist"))
        .output()
        .ok()?;
    let value = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (out.status.success() && !value.is_empty()).then_some(value)
}

/// Converts the app's icon to a small PNG (once; kept in our own cache folder) and returns
/// its `file://` address. Apps that ship their icon only inside an asset catalog get none.
fn icon_png(app: &Path, id: &str, home: &Path) -> Option<String> {
    let cache = home.join("Library/Caches/bytebroom/icons");
    let png = cache.join(format!("{id}.png"));
    if !png.exists() {
        let mut icns = app.join("Contents/Resources").join(plist_value(app, "CFBundleIconFile")?);
        if icns.extension().is_none() {
            icns.set_extension("icns");
        }
        std::fs::create_dir_all(&cache).ok()?;
        let done = Command::new("sips")
            .args(["-s", "format", "png", "-Z", "96"])
            .arg(&icns)
            .arg("--out")
            .arg(&png)
            .output()
            .ok()?;
        if !done.status.success() || !png.exists() {
            return None;
        }
    }
    Some(format!("file://{}", png.display()))
}

fn collect_bundles(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "app") {
            // A link (Safari and other built-in apps) points into the sealed system: it cannot
            // be uninstalled, and its data is not ours to offer.
            if !e.file_type().is_ok_and(|t| t.is_symlink()) {
                out.push(p);
            }
        } else if depth > 0 && e.file_type().is_ok_and(|t| t.is_dir()) {
            collect_bundles(&p, depth - 1, out); // e.g. /Applications/Utilities
        }
    }
}

fn installed(home: &Path) -> Vec<Installed> {
    let mut bundles = Vec::new();
    collect_bundles(Path::new("/Applications"), 1, &mut bundles);
    collect_bundles(&home.join("Applications"), 1, &mut bundles);
    bundles
        .into_par_iter()
        .filter_map(|path| {
            // An identifier without a dot is too vague to match files against.
            let id = plist_value(&path, "CFBundleIdentifier").filter(|id| id.contains('.'))?;
            let name = path.file_stem()?.to_string_lossy().into_owned();
            // "Electron" is what unbranded Electron apps all call themselves, so it names nobody.
            let alt = plist_value(&path, "CFBundleName")
                .filter(|alt| alt.len() >= 3 && alt != &name && alt != "Electron");
            let icon = icon_png(&path, &id, home);
            Some(Installed { name, alt, id, icon, path })
        })
        .collect()
}

/// Does a file or folder called `entry` belong to the app with this bundle identifier?
/// Deliberately narrow: "com.google.Chrome.for.testing" is a different program from
/// "com.google.Chrome", so an arbitrary longer name is NOT a match. Missing a trace is
/// better than deleting another program's files.
fn id_matches(entry: &str, id: &str) -> bool {
    let (e, id) = (entry.to_ascii_lowercase(), id.to_ascii_lowercase());
    if e == id {
        return true;
    }
    // Only app-specific identifiers get the looser forms; a two-part one (com.vendor) could
    // be a whole company's namespace.
    let specific = id.split('.').count() >= 3;
    if let Some(rest) = e.strip_prefix(&format!("{id}.")) {
        // What may follow the identifier: a file type, or one of the standard helper names.
        let next = rest.split('.').next().unwrap_or("");
        let is_uuid = next.len() == 36 && next.matches('-').count() == 4; // ByHost preference files
        return match next {
            "plist" | "savedstate" | "binarycookies" => true,
            "helper" | "shipit" => specific,
            _ => specific && is_uuid,
        };
    }
    // TEAMID.com.vendor.App, group.com.vendor.App
    specific && e.ends_with(&format!(".{id}"))
}

/// Which installed app owns `entry`? The most specific identifier wins, so files of
/// "com.vendor.App.beta" are never handed to "com.vendor.App" while the beta is installed.
pub fn owner(apps: &[Installed], entry: &str, by_name: bool) -> Option<usize> {
    let by_id = (0..apps.len()).filter(|&i| id_matches(entry, &apps[i].id)).max_by_key(|&i| apps[i].id.len());
    by_id.or_else(|| {
        let named = |a: &Installed| {
            entry.eq_ignore_ascii_case(&a.name) || a.alt.as_deref().is_some_and(|alt| entry.eq_ignore_ascii_case(alt))
        };
        by_name.then(|| apps.iter().position(named)).flatten()
    })
}

/// Size from the finished scan; small files the scan did not keep are measured directly.
fn size_of(tree: &Node, path: &Path) -> u64 {
    let mut node = tree;
    for c in path.components().skip(1) {
        let name = c.as_os_str().to_string_lossy();
        match node.children.iter().find(|n| n.name == name) {
            Some(n) => node = n,
            None => return std::fs::symlink_metadata(path).map(|m| m.blocks() * 512).unwrap_or(0),
        }
    }
    node.size
}

/// One group per installed app: the app itself first, then everything it left behind.
pub fn find_apps(tree: &Node, home: &Path) -> Vec<Group> {
    let apps = installed(home);
    let mut traces: Vec<Vec<Finding>> = apps.iter().map(|_| Vec::new()).collect();

    let mut spots: Vec<(PathBuf, &'static str, bool)> = SPOTS
        .iter()
        .map(|&(p, why, by_name)| {
            let dir = match p.strip_prefix("~/") {
                Some(rest) => home.join(rest),
                None => PathBuf::from(p),
            };
            (dir, why, by_name)
        })
        .collect();
    // The per-user temp and cache folders macOS hands to each app.
    if let Ok(tmp) = std::fs::canonicalize(std::env::temp_dir()) {
        if let Some(parent) = tmp.parent() {
            spots.push((parent.join("C"), "Its cache.", false));
        }
        spots.push((tmp, "Its temporary files.", false));
    }

    for (dir, why, by_name) in spots {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let entry = e.file_name().to_string_lossy().into_owned();
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
            let mut push = |i: usize, path: PathBuf, is_dir: bool| {
                traces[i].push(Finding { size: size_of(tree, &path), is_dir, path, why, checked: false, needs_ignore: false });
            };
            if let Some(i) = owner(&apps, &entry, by_name) {
                push(i, e.path(), is_dir);
            } else if by_name && is_dir {
                // Vendor folders: ~/Library/Application Support/Google/Chrome belongs to
                // com.google.Chrome. The folder must be the identifier's vendor part and the
                // item inside must carry the app's exact name.
                let Ok(inner) = std::fs::read_dir(e.path()) else { continue };
                for c in inner.flatten() {
                    let child = c.file_name().to_string_lossy().into_owned();
                    let found = apps.iter().position(|a| {
                        a.id.split('.').nth(1).is_some_and(|vendor| vendor.eq_ignore_ascii_case(&entry))
                            && (child.eq_ignore_ascii_case(&a.name)
                                || a.alt.as_deref().is_some_and(|alt| child.eq_ignore_ascii_case(alt)))
                    });
                    if let Some(i) = found {
                        push(i, c.path(), c.file_type().is_ok_and(|t| t.is_dir()));
                    }
                }
            }
        }
    }

    let mut groups: Vec<Group> = Vec::new();
    for (app, mut left) in apps.into_iter().zip(traces) {
        let icon = app.icon;
        left.sort_unstable_by(|a, b| b.size.cmp(&a.size));
        let mut items = vec![Finding {
            size: size_of(tree, &app.path),
            is_dir: true,
            path: app.path,
            why: "The app itself. Quit it before deleting.",
            checked: false,
            needs_ignore: false,
        }];
        items.append(&mut left);
        // Group names double as selection keys, so two apps with one name need telling apart.
        let name = if groups.iter().any(|g| g.name == app.name) {
            format!("{} ({})", app.name, app.id)
        } else {
            app.name
        };
        let total = items.iter().map(|f| f.size).sum();
        groups.push(Group { name, safe: false, app: true, icon, total, items });
    }
    groups.sort_unstable_by(|a, b| b.total.cmp(&a.total));
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, id: &str) -> Installed {
        Installed { name: name.into(), alt: None, id: id.into(), icon: None, path: PathBuf::new() }
    }

    #[test]
    fn files_go_to_the_right_app_or_to_nobody() {
        let apps = [
            app("Chrome", "com.google.Chrome"),
            app("Chrome Canary", "com.google.Chrome.canary"),
            app("Tool", "com.vendor"),
            app("Notes", "com.example.notes-app"),
        ];
        let owner = |entry: &str, by_name: bool| owner(&apps, entry, by_name).map(|i| apps[i].name.as_str());

        assert_eq!(owner("com.google.Chrome", false), Some("Chrome"));
        assert_eq!(owner("com.google.Chrome.plist", false), Some("Chrome"));
        assert_eq!(owner("com.google.chrome.savedState", false), Some("Chrome"), "case does not matter");
        assert_eq!(owner("EQHXZ8M8AV.com.google.Chrome", false), Some("Chrome"), "team-prefixed group folder");
        // The more specific app wins; plain Chrome must not claim Canary's files.
        assert_eq!(owner("com.google.Chrome.canary", false), Some("Chrome Canary"));
        assert_eq!(owner("com.google.Chrome.canary.plist", false), Some("Chrome Canary"));
        assert_eq!(owner("com.google.Chrome.helper", false), Some("Chrome"));
        assert_eq!(owner("com.google.Chrome.ShipIt.5A99E75A-A660-5200-B206-1E2B52AD94F8.plist", false), Some("Chrome"));
        // A different program that merely shares the prefix belongs to nobody.
        assert_eq!(owner("com.google.chrome.for.testing", false), None);
        assert_eq!(owner("com.google.chrome.for.testing.plist", false), None);
        assert_eq!(owner("com.google.Chrome.oqv7h9", false), None, "random temp names are not claimed");
        // Near misses belong to nobody.
        assert_eq!(owner("com.google.ChromeRemoteDesktop", false), None);
        assert_eq!(owner("com.google.Keystone", false), None);
        // A two-part identifier never claims a whole vendor namespace.
        assert_eq!(owner("com.vendor.plist", false), Some("Tool"));
        assert_eq!(owner("com.vendor.OtherProduct", false), None);
        // Folders named after the app only count where that is allowed.
        assert_eq!(owner("Notes", true), Some("Notes"));
        assert_eq!(owner("Notes", false), None);
        assert_eq!(owner("Notes Backup", true), None);
        // The app's internal name counts too, where names are allowed.
        let code = [Installed { name: "Visual Studio Code".into(), alt: Some("Code".into()), id: "com.microsoft.VSCode".into(), icon: None, path: PathBuf::new() }];
        assert_eq!(super::owner(&code, "Code", true), Some(0));
        assert_eq!(super::owner(&code, "Code", false), None);
    }
}
