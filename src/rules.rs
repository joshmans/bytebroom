//! Decides which scanned folders are worth offering for deletion, and why.

use crate::Node;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// Anything smaller is noise in a "free up space" list.
const MIN_FINDING: u64 = 50 << 20;

// Marker files that prove a folder is a project of some kind (set during the scan).
/// Name of the group whose items the app double-checks against git before listing.
pub const PROJECT_GROUP: &str = "Project build folders";

pub const PKG_JSON: u32 = 1 << 0; // JavaScript / TypeScript
pub const CARGO: u32 = 1 << 1; // Rust
pub const PODFILE: u32 = 1 << 2; // iOS (CocoaPods)
pub const GRADLE: u32 = 1 << 3; // Android / Java / Kotlin
pub const PUBSPEC: u32 = 1 << 4; // Flutter / Dart
pub const PYTHON: u32 = 1 << 5; // a Python project that declares its dependencies
pub const MAVEN: u32 = 1 << 6; // Java (Maven)
pub const SWIFTPM: u32 = 1 << 7; // Swift packages
pub const DOTNET: u32 = 1 << 8; // C# / F#
pub const COMPOSER: u32 = 1 << 9; // PHP
pub const MIX: u32 = 1 << 10; // Elixir
pub const CMAKE: u32 = 1 << 11; // C / C++
pub const ZIG: u32 = 1 << 12;
pub const STACK: u32 = 1 << 13; // Haskell
pub const GODOT: u32 = 1 << 14;
pub const PYVENV: u32 = 1 << 15; // this folder itself IS a Python virtual environment

pub fn marker(file_name: &str) -> u32 {
    match file_name {
        "package.json" => PKG_JSON,
        "Cargo.toml" => CARGO,
        "Podfile" => PODFILE,
        "build.gradle" | "build.gradle.kts" | "settings.gradle" | "settings.gradle.kts" => GRADLE,
        "pubspec.yaml" => PUBSPEC,
        "pyproject.toml" | "requirements.txt" | "Pipfile" | "setup.py" | "uv.lock" | "poetry.lock" => PYTHON,
        "pom.xml" => MAVEN,
        "Package.swift" => SWIFTPM,
        "composer.json" => COMPOSER,
        "mix.exs" => MIX,
        "CMakeLists.txt" => CMAKE,
        "build.zig" => ZIG,
        "stack.yaml" => STACK,
        "project.godot" => GODOT,
        "pyvenv.cfg" => PYVENV,
        _ => match Path::new(file_name).extension().and_then(|e| e.to_str()) {
            Some("csproj" | "fsproj" | "vbproj" | "sln") => DOTNET,
            _ => 0,
        },
    }
}

pub struct Finding {
    pub path: PathBuf,
    pub size: u64,
    pub is_dir: bool,
    pub why: &'static str,
    pub checked: bool,
    /// Ambiguous folder name (`dist`, `bin`, …): only list it if git says it is ignored.
    pub needs_ignore: bool,
}

pub struct Group {
    pub name: String,
    /// An installed app and its leftovers, shown in the Apps view (never "safe").
    pub app: bool,
    /// `file://` address of the app's icon as a PNG, when one could be extracted.
    pub icon: Option<String>,
    /// true: comes back by itself or with a rebuild. false: you lose something, so look first.
    pub safe: bool,
    pub total: u64,
    pub items: Vec<Finding>,
}

const APP_CACHE: usize = 0;
const PKG_CACHE: usize = 1;
const PROJECT: usize = 2;
const XCODE: usize = 3;
const LOGS: usize = 4;
const TRASH: usize = 5;
const SIMULATORS: usize = 6;
const EMULATORS: usize = 7;
const VMS: usize = 8;
const DOWNLOADS: usize = 9;
const TEMP: usize = 10;
const BACKUPS: usize = 11;
const CHECK_CACHE: usize = 12;

const GROUPS: &[(&str, bool)] = &[
    ("App caches", true),
    ("Package manager caches", true),
    ("Project build folders", true),
    ("Xcode build data", true),
    ("Logs", true),
    ("Trash", false),
    ("iOS simulators", false),
    ("Android emulators", false),
    ("Docker and VM data", false),
    ("Downloads", false),
    ("Temporary files", false),
    ("Backups and archives", false),
    ("Caches worth checking", false),
];

#[derive(Clone, Copy)]
struct Rule {
    needs_ignore: bool,
    group: usize,
    /// true: every item inside is its own finding. false: the folder itself is the finding.
    each_child: bool,
    why: &'static str,
}

// Known locations. `~` is the user's home folder.
const PATH_RULES: &[(&str, usize, bool, &str)] = &[
    ("~/Library/Caches", APP_CACHE, true, "App cache. The app rebuilds it when needed."),
    ("/Library/Caches", APP_CACHE, true, "System-wide app cache. Rebuilt when needed (needs admin)."),
    // ~/.cache is a free-for-all: some tools keep downloaded models or login sessions there,
    // so only the entries known to be pure download caches count as safe.
    ("~/.cache", CHECK_CACHE, true, "Tool folder. Usually re-downloaded, but some tools keep models or logins here."),
    ("~/.cache/uv", PKG_CACHE, false, "uv download cache. Refilled on the next install."),
    ("~/.cache/pip", PKG_CACHE, false, "pip download cache. Refilled on the next install."),
    ("~/.cache/yarn", PKG_CACHE, false, "Yarn download cache. Refilled on the next install."),
    ("~/.cache/pnpm", PKG_CACHE, false, "pnpm download cache. Refilled on the next install."),
    ("~/.cache/prisma", PKG_CACHE, false, "Prisma engine downloads. Fetched again when needed."),
    ("~/.cache/node", PKG_CACHE, false, "Node tool downloads. Fetched again when needed."),
    ("~/.cache/go-build", PKG_CACHE, false, "Go build cache. Rebuilt on the next build."),
    ("~/Library/Caches/ms-playwright", APP_CACHE, false, "Playwright's browsers. `npx playwright install` downloads them again."),
    // Listed so their children are judged one by one: IDE folders in here are demoted in `add`.
    ("~/Library/Caches/Google", APP_CACHE, true, "App cache. The app rebuilds it when needed."),
    ("~/Library/Caches/JetBrains", APP_CACHE, true, "App cache. The app rebuilds it when needed."),
    ("~/.gradle/caches", PKG_CACHE, false, "Gradle downloads. Fetched again on the next build."),
    ("~/.gradle/wrapper/dists", PKG_CACHE, false, "Gradle versions. Downloaded again when a project needs one."),
    ("~/.npm/_cacache", PKG_CACHE, false, "npm download cache. Refilled on the next install."),
    ("~/.yarn/berry/cache", PKG_CACHE, false, "Yarn download cache. Refilled on the next install."),
    ("~/.bun/install/cache", PKG_CACHE, false, "Bun download cache. Refilled on the next install."),
    // Not the whole of ~/.pub-cache: its bin/ and global_packages/ are installed tools.
    ("~/.pub-cache/hosted", PKG_CACHE, false, "Dart/Flutter packages. `flutter pub get` fetches them again."),
    ("~/.pub-cache/git", PKG_CACHE, false, "Dart/Flutter git packages. `flutter pub get` fetches them again."),
    ("~/.cargo/registry", PKG_CACHE, false, "Rust crate downloads. Fetched again on the next build."),
    ("~/go/pkg/mod", PKG_CACHE, false, "Go module cache. Remove with `go clean -modcache`; refetched on build."),
    ("~/.m2/repository", CHECK_CACHE, false, "Maven repository. Downloads come back, but anything you installed with `mvn install` does not."),
    // Only the public spec repos: private ones added with `pod repo add` would need re-adding.
    ("~/.cocoapods/repos/trunk", PKG_CACHE, false, "CocoaPods public specs. `pod install` fetches them again."),
    ("~/.cocoapods/repos/master", PKG_CACHE, false, "CocoaPods public specs. `pod install` fetches them again."),
    ("~/Library/pnpm/store", PKG_CACHE, false, "pnpm package store. Refilled on the next install."),
    ("~/.dartServer", PKG_CACHE, false, "Dart analysis cache. Rebuilt when you open a project."),
    ("~/Library/Developer/Xcode/DerivedData", XCODE, true, "Xcode build output. Recreated on the next build."),
    ("~/Library/Developer/Xcode/iOS DeviceSupport", XCODE, true, "Debug symbols for one iOS version. Re-downloaded when a device connects."),
    ("~/Library/Developer/Xcode/watchOS DeviceSupport", XCODE, true, "Debug symbols for one watchOS version. Re-downloaded when a device connects."),
    ("~/Library/Developer/Xcode/DocumentationCache", XCODE, false, "Xcode documentation cache. Rebuilt when needed."),
    ("~/Library/Developer/CoreSimulator/Caches", XCODE, false, "Simulator cache. Rebuilt when needed."),
    ("~/Library/Logs", LOGS, true, "Old log files. Nothing depends on them."),
    ("~/.Trash", TRASH, true, "Already in the Trash. Deleting it here is permanent."),
    ("~/Library/Developer/CoreSimulator/Devices", SIMULATORS, false, "Simulator devices. Apps and data installed in them are lost. `xcrun simctl delete unavailable` removes only the dead ones."),
    ("/Library/Developer/CoreSimulator/Images", SIMULATORS, true, "A simulator OS download. Xcode can download it again."),
    ("~/.android/avd", EMULATORS, true, "An Android emulator. Its apps and data are lost; it can be recreated."),
    ("~/.Genymobile", EMULATORS, false, "Genymotion emulators. Their apps and data are lost."),
    ("~/.colima", VMS, false, "Colima VM disk: all Docker images, containers and volumes in it are lost."),
    ("~/.lima", VMS, true, "A Lima VM. Everything inside it is lost."),
    ("~/.orbstack", VMS, false, "OrbStack data: Docker images, containers and volumes are lost."),
    ("~/Library/Containers/com.docker.docker", VMS, false, "Docker Desktop data: images, containers and volumes are lost."),
    ("~/Downloads", DOWNLOADS, true, "A download. Delete it if you no longer need the file."),
    ("/private/tmp", TEMP, true, "Temporary files. Could be in use by a running app; macOS clears this on restart."),
    ("~/Library/Application Support/MobileSync/Backup", BACKUPS, true, "An iPhone/iPad backup. Gone for good once deleted."),
    ("~/Library/Developer/Xcode/Archives", BACKUPS, true, "Archived app builds. Needed to symbolicate crashes of released versions."),
];

// Folders inside a project that a build or install recreates: (folder name, project kinds,
// needs git-ignore proof, reason). A rule only matches when the parent holds that kind's
// marker file, so a stray folder that happens to share the name is left alone. Names that
// people also use for real work (`dist`, `bin`, `out`…) additionally need git to confirm
// the folder is ignored; without a git repository they are not listed at all.
const PROJECT_RULES: &[(&str, u32, bool, &str)] = &[
    // JavaScript / TypeScript
    ("node_modules", PKG_JSON, false, "Installed JS packages. `npm install` restores them."),
    (".next", PKG_JSON, false, "Next.js build output. Recreated on the next build."),
    (".nuxt", PKG_JSON, false, "Nuxt build output. Recreated on the next build."),
    (".output", PKG_JSON, false, "Nuxt/Nitro build output. Recreated on the next build."),
    (".svelte-kit", PKG_JSON, false, "SvelteKit build output. Recreated on the next build."),
    (".angular", PKG_JSON, false, "Angular build cache. Recreated on the next build."),
    (".turbo", PKG_JSON, false, "Turborepo cache. Recreated on the next build."),
    (".nx", PKG_JSON, false, "Nx cache. Recreated on the next build."),
    (".parcel-cache", PKG_JSON, false, "Parcel cache. Recreated on the next build."),
    (".expo", PKG_JSON, false, "Expo cache. Recreated on the next start."),
    (".pnpm-store", PKG_JSON, false, "pnpm package store. Refilled on the next install."),
    ("dist", PKG_JSON, true, "Build output (ignored by git). Recreated on the next build."),
    ("build", PKG_JSON, true, "Build output (ignored by git). Recreated on the next build."),
    // Deliberately absent: `out` and `.cache`. Projects use those names for their own generated
    // data (renders, downloads, API results), which a rebuild does not bring back.
    ("coverage", PKG_JSON, true, "Test coverage report (ignored by git). Recreated on the next test run."),
    // Rust
    ("target", CARGO, false, "Rust build output. `cargo build` recreates it."),
    // iOS / Swift
    ("Pods", PODFILE, false, "Installed CocoaPods. `pod install` restores them."),
    (".build", SWIFTPM, false, "Swift package build output. `swift build` recreates it."),
    // Android / Java / Kotlin / Flutter
    ("build", GRADLE | PUBSPEC | PODFILE, false, "Build output. Recreated on the next build."),
    (".gradle", GRADLE, false, "Project Gradle cache. Recreated on the next build."),
    (".cxx", GRADLE, false, "Android native build cache. Recreated on the next build."),
    (".dart_tool", PUBSPEC, false, "Dart tool cache. `flutter pub get` recreates it."),
    ("target", MAVEN, false, "Maven build output. `mvn package` recreates it."),
    // Python
    (".tox", PYTHON, false, "tox test environments. Recreated on the next tox run."),
    (".mypy_cache", PYTHON, false, "mypy cache. Recreated on the next check."),
    (".pytest_cache", PYTHON, false, "pytest cache. Recreated on the next test run."),
    (".ruff_cache", PYTHON, false, "Ruff cache. Recreated on the next check."),
    // C# / F#
    ("bin", DOTNET, true, ".NET build output (ignored by git). `dotnet build` recreates it."),
    ("obj", DOTNET, true, ".NET build files (ignored by git). `dotnet build` recreates them."),
    // PHP, Elixir
    ("vendor", COMPOSER, false, "Installed PHP packages. `composer install` restores them."),
    ("_build", MIX, false, "Elixir build output. `mix compile` recreates it."),
    ("deps", MIX, false, "Elixir dependencies. `mix deps.get` restores them."),
    // C / C++, Zig, Haskell, Godot
    ("build", CMAKE, true, "CMake build folder (ignored by git). Recreated on the next build."),
    ("cmake-build-debug", CMAKE, false, "CLion build folder. Recreated on the next build."),
    ("cmake-build-release", CMAKE, false, "CLion build folder. Recreated on the next build."),
    (".zig-cache", ZIG, false, "Zig build cache. Recreated on the next build."),
    ("zig-cache", ZIG, false, "Zig build cache. Recreated on the next build."),
    ("zig-out", ZIG, true, "Zig build output (ignored by git). Recreated on the next build."),
    (".stack-work", STACK, false, "Haskell Stack build output. `stack build` recreates it."),
    (".godot", GODOT, false, "Godot import cache. Rebuilt when the project is opened."),
];

const CACHE_DIR_NAMES: &[&str] = &["Cache", "Caches", "Code Cache", "GPUCache", "CachedData"];

struct Walker {
    home: PathBuf,
    exact: HashMap<PathBuf, Rule>,
    groups: Vec<Group>,
}

impl Walker {
    fn add(&mut self, node: &Node, path: PathBuf, mut rule: Rule) {
        if node.size < MIN_FINDING {
            return;
        }
        let caches = self.home.join("Library/Caches");
        let parent = path.parent();
        if parent == Some(caches.join("JetBrains").as_path())
            || (parent == Some(caches.join("Google").as_path()) && node.name.starts_with("AndroidStudio"))
        {
            rule.group = CHECK_CACHE;
            rule.why = "IDE cache. Also holds the IDE's Local History of your edits, which is lost.";
        }
        let g = &mut self.groups[rule.group];
        g.total += node.size;
        g.items.push(Finding { path, size: node.size, is_dir: node.is_dir, why: rule.why, checked: false, needs_ignore: rule.needs_ignore });
    }

    fn path_rule(&self, path: &Path) -> Option<Rule> {
        if let Some(r) = self.exact.get(path) {
            return Some(*r);
        }
        // Per-user system folders: /private/var/folders/xx/yyyy/{C,T,X}
        let rest = path.strip_prefix("/private/var/folders").ok()?;
        if rest.components().count() != 3 {
            return None;
        }
        match rest.file_name()?.to_str()? {
            "C" => Some(Rule { needs_ignore: false, group: APP_CACHE, each_child: true, why: "App cache. The app rebuilds it when needed." }),
            "T" => Some(Rule { needs_ignore: false, group: TEMP, each_child: true, why: "Temporary files. Could be in use by a running app; macOS clears this on restart." }),
            "X" => Some(Rule { needs_ignore: false, group: TEMP, each_child: true, why: "Copy macOS makes of a running app. Usually shares space with the app, so the real size is smaller; removed when the app quits." }),
            _ => None,
        }
    }

    /// `in_projects` is true only inside the user's own folders (not ~/Library, not ~/.tools),
    /// where deleting node_modules and friends cannot break an installed program.
    fn visit(&mut self, node: &Node, path: &Path, parent_markers: u32, in_projects: bool) {
        if let Some(rule) = self.path_rule(path) {
            if rule.each_child {
                for c in &node.children {
                    let child = path.join(&c.name);
                    if self.exact.contains_key(&child) {
                        self.visit(c, &child, 0, false); // it has a more specific rule of its own
                    } else {
                        self.add(c, child, rule);
                    }
                }
            } else {
                self.add(node, path.to_path_buf(), rule);
            }
            return;
        }
        if in_projects {
            let hit = PROJECT_RULES.iter().find(|(n, m, _, _)| *n == node.name && m & parent_markers != 0);
            if let Some(&(_, _, needs_ignore, why)) = hit {
                self.add(node, path.to_path_buf(), Rule { needs_ignore, group: PROJECT, each_child: false, why });
                return;
            }
            // A Python virtual environment is identified by the pyvenv.cfg inside it, whatever it
            // is called. Only safe when the project lists its dependencies, so it can be rebuilt.
            if node.markers & PYVENV != 0 && parent_markers & PYTHON != 0 {
                let why = "Python virtual environment. Recreate it from the project's dependency file.";
                self.add(node, path.to_path_buf(), Rule { needs_ignore: false, group: PROJECT, each_child: false, why });
                return;
            }
        }
        if CACHE_DIR_NAMES.contains(&node.name.as_str())
            && (path.starts_with(self.home.join("Library/Application Support"))
                || path.starts_with(self.home.join("Library/Containers")))
        {
            let why = "App cache. The app rebuilds it when needed.";
            self.add(node, path.to_path_buf(), Rule { needs_ignore: false, group: APP_CACHE, each_child: false, why });
            return;
        }
        let at_home = path == self.home;
        for c in node.children.iter().filter(|c| c.is_dir) {
            let zone = if at_home {
                !c.name.starts_with('.') && c.name != "Library" && c.name != "Applications"
            } else {
                in_projects
            };
            self.visit(c, &path.join(&c.name), node.markers, zone);
        }
    }
}

/// Returns the non-empty groups: safe ones first, each list largest-first.
pub fn classify(root: &Node, base: &Path, home: &Path) -> Vec<Group> {
    let exact = PATH_RULES
        .iter()
        .map(|&(p, group, each_child, why)| {
            let path = match p.strip_prefix("~/") {
                Some(rest) => home.join(rest),
                None => PathBuf::from(p),
            };
            (path, Rule { needs_ignore: false, group, each_child, why })
        })
        .collect();
    let groups = GROUPS
        .iter()
        .map(|&(name, safe)| Group { name: name.to_owned(), safe, app: false, icon: None, total: 0, items: Vec::new() })
        .collect();
    let mut w = Walker { home: home.to_path_buf(), exact, groups };
    // A scan rooted inside the user's folders is project territory from the start.
    let in_projects = base.strip_prefix(home).is_ok_and(|rest| {
        rest.components().next().is_some_and(|c| {
            let c = c.as_os_str().to_string_lossy();
            !c.starts_with('.') && c != "Library" && c != "Applications"
        })
    });
    w.visit(root, base, 0, in_projects);
    let mut groups = w.groups;
    groups.retain(|g| !g.items.is_empty());
    for g in &mut groups {
        g.items.sort_unstable_by(|a, b| b.size.cmp(&a.size));
    }
    groups.sort_by(|a, b| b.safe.cmp(&a.safe).then(b.total.cmp(&a.total)));
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str, markers: u32, children: Vec<Node>) -> Node {
        let size = children.iter().map(|c| c.size).sum();
        Node { name: name.into(), size, is_dir: true, markers, children }
    }
    fn blob(name: &str, mb: u64) -> Node {
        Node { name: name.into(), size: mb << 20, is_dir: false, markers: 0, children: vec![] }
    }

    #[test]
    fn only_truly_regenerable_things_are_safe() {
        let home = dir("me", 0, vec![
            dir("code", 0, vec![dir("app", PKG_JSON, vec![
                dir("node_modules", 0, vec![blob("x", 100)]),
                dir("src", 0, vec![blob("video", 500)]),
            ])]),
            // Same folder name, but it belongs to an installed tool: must be left alone.
            dir(".vscode", 0, vec![dir("ext", PKG_JSON, vec![dir("node_modules", 0, vec![blob("x", 100)])])]),
            dir("py", PYTHON, vec![
                dir("myenv", PYVENV, vec![blob("x", 300)]),
                dir(".mypy_cache", 0, vec![blob("x", 80)]),
            ]),
            // A virtual environment with no dependency file beside it cannot be rebuilt.
            dir("scratch", 0, vec![dir("venv", PYVENV, vec![blob("x", 300)])]),
            dir("web", PKG_JSON, vec![dir("dist", 0, vec![blob("x", 100)])]),
            dir("php", COMPOSER, vec![dir("vendor", 0, vec![blob("x", 100)])]),
            dir("cs", DOTNET, vec![dir("obj", 0, vec![blob("x", 100)])]),
            // No package.json beside it: not provably a project folder.
            dir("stuff", 0, vec![dir("node_modules", 0, vec![blob("x", 100)])]),
            dir("Library", 0, vec![dir("Caches", 0, vec![
                dir("Yarn", 0, vec![blob("x", 200)]),
                dir("tiny", 0, vec![blob("x", 1)]),
                dir("Google", 0, vec![
                    dir("Chrome", 0, vec![blob("x", 300)]),
                    dir("AndroidStudio2026.1", 0, vec![blob("x", 300)]),
                ]),
            ])]),
            dir(".cache", 0, vec![
                dir("uv", 0, vec![blob("x", 100)]),
                dir("huggingface", 0, vec![blob("model", 4000)]),
            ]),
            dir(".pub-cache", 0, vec![
                dir("hosted", 0, vec![blob("x", 100)]),
                dir("bin", 0, vec![blob("tool", 100)]),
            ]),
            dir(".Trash", 0, vec![blob("old.mov", 700)]),
            dir("Downloads", 0, vec![blob("installer.dmg", 900)]),
        ]);
        let root = dir("/", 0, vec![dir("Users", 0, vec![home])]);
        let groups = classify(&root, Path::new("/"), Path::new("/Users/me"));

        // (is it listed as safe?) for a path, or None when it is not listed at all.
        let verdict = |p: &str| -> Option<bool> {
            groups.iter().find(|g| g.items.iter().any(|f| f.path == Path::new(p))).map(|g| g.safe)
        };
        for safe in [
            "/Users/me/code/app/node_modules",
            "/Users/me/Library/Caches/Yarn",
            "/Users/me/Library/Caches/Google/Chrome",
            "/Users/me/.cache/uv",
            "/Users/me/.pub-cache/hosted",
            "/Users/me/py/myenv",
            "/Users/me/py/.mypy_cache",
            "/Users/me/php/vendor",
            "/Users/me/web/dist",
            "/Users/me/cs/obj",
        ] {
            assert_eq!(verdict(safe), Some(true), "{safe} should be safe");
        }
        for look_first in [
            "/Users/me/Library/Caches/Google/AndroidStudio2026.1", // holds Local History
            "/Users/me/.cache/huggingface",                        // downloaded models
            "/Users/me/.Trash/old.mov",                            // permanent once deleted here
            "/Users/me/Downloads/installer.dmg",
        ] {
            assert_eq!(verdict(look_first), Some(false), "{look_first} must not be called safe");
        }
        for unlisted in [
            "/Users/me/.vscode/ext/node_modules",
            "/Users/me/stuff/node_modules",
            "/Users/me/scratch/venv",
            "/Users/me/.pub-cache/bin", // installed tools, not cache
            "/Users/me/.pub-cache",
            "/Users/me/code/app/src",
            "/Users/me/Library/Caches/tiny",
        ] {
            assert_eq!(verdict(unlisted), None, "{unlisted} must not be listed");
        }
        // Ambiguous names are flagged so the app asks git before showing them.
        let needs_ignore = |p: &str| groups.iter().flat_map(|g| &g.items).find(|f| f.path == Path::new(p)).unwrap().needs_ignore;
        assert!(needs_ignore("/Users/me/web/dist") && needs_ignore("/Users/me/cs/obj"));
        assert!(!needs_ignore("/Users/me/code/app/node_modules") && !needs_ignore("/Users/me/php/vendor"));
        assert_eq!(marker("App.csproj"), DOTNET);
        let first_look = groups.iter().position(|g| !g.safe).unwrap();
        assert!(groups[first_look..].iter().all(|g| !g.safe), "safe groups come first");
    }
}
