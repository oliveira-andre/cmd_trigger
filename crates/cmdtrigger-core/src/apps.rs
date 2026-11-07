//! The apps on this computer, each with the line that opens it, for the command form. The form
//! saves that line as an ordinary command, so nothing else needs to know about apps.
//!
//! - macOS: `.app` bundles in the Applications folders, opened with `open -a`.
//! - Linux: `.desktop` entries, opened with their own `Exec` line.
//! - Windows: the Start menu through `Get-StartApps`, which has store apps too, opened through
//!   `shell:AppsFolder`.

use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct InstalledApp {
    pub name: String,
    pub command: String,
}

/// Sorted by name, one entry per name.
pub async fn installed() -> Vec<InstalledApp> {
    let mut apps = find().await;
    apps.sort_by_cached_key(|a| a.name.to_lowercase());
    apps.dedup_by_key(|a| a.name.to_lowercase());
    apps
}

#[cfg(target_os = "macos")]
async fn find() -> Vec<InstalledApp> {
    let mut roots: Vec<std::path::PathBuf> =
        ["/Applications", "/System/Applications", "/System/Library/CoreServices/Applications"].map(Into::into).into();
    roots.extend(dirs::home_dir().map(|home| home.join("Applications")));
    let mut apps = tokio::task::spawn_blocking(move || bundles(&roots)).await.unwrap_or_default();
    apps.push(open_by_name("Finder"));
    apps
}

/// Every `.app` in these folders. Suites and web apps sit in subfolders, so this looks a few
/// levels down, but never inside an app: those hold helpers nobody opens.
#[cfg(any(target_os = "macos", test))]
fn bundles(roots: &[std::path::PathBuf]) -> Vec<InstalledApp> {
    fn walk(folder: &std::path::Path, levels: usize, apps: &mut Vec<InstalledApp>) {
        for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "app") {
                let name = path.file_stem().and_then(|n| n.to_str()).unwrap_or_default();
                if !name.is_empty() && !name.starts_with('.') {
                    apps.push(open_by_name(name));
                }
            } else if levels > 0 && path.is_dir() {
                walk(&path, levels - 1, apps);
            }
        }
    }
    let mut apps = Vec::new();
    for root in roots {
        walk(root, 3, &mut apps);
    }
    apps
}

#[cfg(any(target_os = "macos", test))]
fn open_by_name(name: &str) -> InstalledApp {
    InstalledApp { name: name.to_string(), command: format!("open -a {}", quote(name)) }
}

/// For `/bin/sh`: plain words stay as they are, the rest go in double quotes.
#[cfg(any(target_os = "macos", test))]
fn quote(text: &str) -> String {
    if text.chars().all(|c| c.is_ascii_alphanumeric() || "._-+".contains(c)) {
        return text.to_string();
    }
    let mut quoted = String::from("\"");
    for c in text.chars() {
        if matches!(c, '"' | '\\' | '$' | '`') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    quoted
}

#[cfg(target_os = "linux")]
async fn find() -> Vec<InstalledApp> {
    use std::path::PathBuf;
    let mut data: Vec<PathBuf> = Vec::new();
    if let Some(home) = dirs::data_dir() {
        data.push(home.join("flatpak/exports/share"));
        data.insert(0, home);
    }
    let system = std::env::var("XDG_DATA_DIRS").ok().filter(|d| !d.is_empty());
    data.extend(system.as_deref().unwrap_or("/usr/local/share:/usr/share").split(':').map(PathBuf::from));
    data.extend(["/var/lib/flatpak/exports/share", "/var/lib/snapd/desktop"].map(PathBuf::from));
    let folders: Vec<PathBuf> = data.iter().map(|d| d.join("applications")).collect();
    tokio::task::spawn_blocking(move || desktop_entries(&folders)).await.unwrap_or_default()
}

/// An entry replaces any of the same file name in later folders, even when it only hides them.
#[cfg(any(target_os = "linux", test))]
fn desktop_entries(folders: &[std::path::PathBuf]) -> Vec<InstalledApp> {
    let mut seen = std::collections::HashSet::new();
    let mut apps = Vec::new();
    for folder in folders {
        for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "desktop") && seen.insert(entry.file_name()) {
                apps.extend(std::fs::read_to_string(&path).ok().and_then(|text| desktop_entry(&text)));
            }
        }
    }
    apps
}

/// None for what a launcher wouldn't list. Apps that need a terminal are left out too: a run has
/// none.
#[cfg(any(target_os = "linux", test))]
fn desktop_entry(text: &str) -> Option<InstalledApp> {
    let (mut in_entry, mut is_app, mut name, mut exec) = (false, false, None, None);
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
        } else if let (true, Some((key, value))) = (in_entry, line.split_once('=')) {
            match (key.trim(), value.trim()) {
                ("Type", "Application") => is_app = true,
                ("Name", value) => name = Some(value),
                ("Exec", value) => exec = Some(value),
                ("NoDisplay" | "Hidden" | "Terminal", "true") => return None,
                _ => {}
            }
        }
    }
    let (name, command) = (name?, exec_line(exec?));
    (is_app && !command.is_empty()).then(|| InstalledApp { name: name.to_string(), command })
}

/// An `Exec` line without its placeholders for files and URLs, or flatpak's `@@` markers.
#[cfg(any(target_os = "linux", test))]
fn exec_line(exec: &str) -> String {
    let words: Vec<&str> =
        exec.split_whitespace().filter(|w| !w.starts_with("@@") && !w.replace("%%", "").contains('%')).collect();
    words.join(" ").replace("%%", "%")
}

#[cfg(windows)]
async fn find() -> Vec<InstalledApp> {
    let script = "[Console]::OutputEncoding = [Text.Encoding]::UTF8; Get-StartApps | Select-Object Name, AppID | ConvertTo-Json -Compress";
    let text = crate::monitors::output("powershell", &["-NoProfile", "-Command", script]).await;
    text.map(|text| start_apps(&text)).unwrap_or_default()
}

/// `ConvertTo-Json` writes a bare object when there's only one. `start` rather than `explorer`,
/// which exits with 1 even when the app opens.
#[cfg(any(windows, test))]
fn start_apps(json: &str) -> Vec<InstalledApp> {
    let value: serde_json::Value = serde_json::from_str(json.trim_start_matches('\u{feff}')).unwrap_or_default();
    let entries = match value {
        serde_json::Value::Array(list) => list,
        other => vec![other],
    };
    let app = |e: &serde_json::Value| {
        let (name, id) = (e["Name"].as_str()?, e["AppID"].as_str()?);
        Some(InstalledApp { name: name.to_string(), command: format!("start \"\" \"shell:AppsFolder\\{id}\"") })
    };
    entries.iter().filter_map(app).collect()
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
async fn find() -> Vec<InstalledApp> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn names(apps: &[InstalledApp]) -> Vec<&str> {
        let mut names: Vec<&str> = apps.iter().map(|a| a.name.as_str()).collect();
        names.sort();
        names
    }

    #[test]
    fn plain_names_stay_plain_and_the_rest_are_quoted() {
        assert_eq!(open_by_name("Calculator").command, "open -a Calculator");
        assert_eq!(open_by_name("Google Chrome").command, r#"open -a "Google Chrome""#);
        assert_eq!(open_by_name("Bob's $App `x` \"y\"").command, r#"open -a "Bob's \$App \`x\` \"y\"""#);
        assert_eq!(open_by_name("Câmera").command, r#"open -a "Câmera""#);
    }

    #[test]
    fn finds_bundles_but_not_what_is_inside_them() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Applications");
        for folder in [
            "Calculator.app/Contents/Helpers/Helper.app",
            "Utilities/Terminal.app",
            "Chrome Apps.localized/You Tube.app",
            "One/Two/Three/Deep.app",
            "One/Two/Three/Four/TooDeep.app",
            ".Hidden.app",
        ] {
            fs::create_dir_all(root.join(folder)).unwrap();
        }
        fs::write(root.join("notes.txt"), "").unwrap();
        let elsewhere = dir.path().join("Cryptex/Safari.app");
        fs::create_dir_all(&elsewhere).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&elsewhere, root.join("Safari.app")).unwrap();

        let apps = bundles(&[root, dir.path().join("missing")]);
        let mut want = vec!["Calculator", "Deep", "Terminal", "You Tube"];
        if cfg!(unix) {
            want.push("Safari");
            want.sort();
        }
        assert_eq!(names(&apps), want);
    }

    #[test]
    fn reads_desktop_entries() {
        let firefox = "[Desktop Entry]\nType=Application\nName=Firefox\nName[pt]=Raposa\nExec=firefox %u\n\n[Desktop Action new]\nName=New Window\nExec=firefox --new-window\n";
        assert_eq!(desktop_entry(firefox), Some(InstalledApp { name: "Firefox".into(), command: "firefox".into() }));
        for hidden in ["NoDisplay=true", "Hidden=true", "Terminal=true"] {
            assert_eq!(desktop_entry(&format!("[Desktop Entry]\nType=Application\nName=X\nExec=x\n{hidden}\n")), None);
        }
        assert_eq!(desktop_entry("[Desktop Entry]\nType=Link\nName=X\nExec=x\n"), None);
        assert_eq!(desktop_entry("[Desktop Entry]\nType=Application\nName=X\n"), None);
        assert_eq!(desktop_entry("[Other]\nType=Application\nName=X\nExec=x\n"), None);
    }

    #[test]
    fn exec_lines_lose_their_placeholders() {
        assert_eq!(exec_line("gnome-calculator"), "gnome-calculator");
        assert_eq!(exec_line("code --new-window %F"), "code --new-window");
        assert_eq!(exec_line("app --url=%u --rate=50%%"), "app --rate=50%");
        assert_eq!(
            exec_line("/usr/bin/flatpak run --branch=stable --command=foo org.foo.Foo @@u %U @@"),
            "/usr/bin/flatpak run --branch=stable --command=foo org.foo.Foo"
        );
    }

    #[test]
    fn an_earlier_desktop_entry_wins_even_to_hide_one() {
        let dir = tempfile::tempdir().unwrap();
        let (user, system) = (dir.path().join("user"), dir.path().join("system"));
        fs::create_dir_all(&user).unwrap();
        fs::create_dir_all(&system).unwrap();
        let entry = |name: &str| format!("[Desktop Entry]\nType=Application\nName={name}\nExec=x\n");
        fs::write(user.join("a.desktop"), entry("Mine")).unwrap();
        fs::write(system.join("a.desktop"), entry("System")).unwrap();
        fs::write(user.join("b.desktop"), "[Desktop Entry]\nHidden=true\n").unwrap();
        fs::write(system.join("b.desktop"), entry("Hidden one")).unwrap();
        fs::write(system.join("c.desktop"), entry("Other")).unwrap();
        fs::write(system.join("notes.txt"), entry("Not an entry")).unwrap();
        assert_eq!(names(&desktop_entries(&[user, system])), ["Mine", "Other"]);
    }

    #[test]
    fn reads_the_start_menu() {
        let many = r#"[{"Name":"Calculator","AppID":"Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"},{"Name":"Chrome","AppID":"Chrome"}]"#;
        let apps = start_apps(many);
        assert_eq!(apps.len(), 2);
        assert_eq!(apps[0].command, r#"start "" "shell:AppsFolder\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App""#);
        assert_eq!(start_apps("\u{feff}{\"Name\":\"One\",\"AppID\":\"id\"}").len(), 1);
        assert!(start_apps("").is_empty());
        assert!(start_apps("not json").is_empty());
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn lists_this_macs_apps() {
        let apps = installed().await;
        let find = |name: &str| apps.iter().find(|a| a.name == name);
        assert_eq!(find("Calculator").unwrap().command, "open -a Calculator");
        assert!(find("Safari").is_some(), "a symlink into the system's cryptex");
        assert!(find("Finder").is_some());
        let lower: Vec<String> = apps.iter().map(|a| a.name.to_lowercase()).collect();
        assert!(lower.windows(2).all(|w| w[0] < w[1]), "sorted, with no repeats");
    }
}
