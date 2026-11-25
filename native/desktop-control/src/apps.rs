//! Installed apps by the names people use ("Cursor", "Claude", "Notepad"), from the same Start menu
//! list as `Get-StartApps`, so `launch` opens the app itself instead of a same-named command-line
//! tool on PATH, and the model never has to look app IDs up with a shell.

use windows::Win32::{
    System::Com::CoTaskMemFree,
    UI::Shell::{
        BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, KF_FLAG_DEFAULT,
        SHGetKnownFolderItem, SIGDN, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING,
    },
};

pub(crate) struct App {
    pub name: String,
    /// The Start menu ID, opened as `shell:AppsFolder\<id>`.
    pub id: String,
}

/// Every app in the Start menu's All apps list.
pub(crate) fn installed() -> Vec<App> {
    let Ok(folder) = (unsafe {
        SHGetKnownFolderItem::<IShellItem>(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None)
    }) else {
        return Vec::new();
    };
    let Ok(items) = (unsafe { folder.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems) })
    else {
        return Vec::new();
    };
    let mut apps = Vec::new();
    loop {
        let mut item = [None];
        let mut fetched = 0;
        if unsafe { items.Next(&mut item, Some(&mut fetched)) }.is_err() || fetched == 0 {
            break;
        }
        let Some(item) = item[0].take() else {
            break;
        };
        if let (Some(name), Some(id)) = (
            display(&item, SIGDN_NORMALDISPLAY),
            display(&item, SIGDN_PARENTRELATIVEPARSING),
        ) {
            apps.push(App { name, id });
        }
    }
    apps
}

fn display(item: &IShellItem, kind: SIGDN) -> Option<String> {
    let text = unsafe { item.GetDisplayName(kind) }.ok()?;
    let value = unsafe { text.to_string() }.ok();
    unsafe { CoTaskMemFree(Some(text.0 as *const _)) };
    value.filter(|value| !value.is_empty())
}

/// The app a name means: an exact name first, then the shortest name starting with it, then the
/// shortest containing it. Paths, URLs and shell locations are not app names.
pub(crate) fn pick<'a>(apps: &'a [App], query: &str) -> Option<&'a App> {
    let query = query.trim().to_lowercase();
    if query.is_empty() || query.contains([':', '\\', '/']) {
        return None;
    }
    let query = query.trim_end_matches(".exe");
    // Uninstallers, manuals and web links share their app's name.
    let candidates = apps
        .iter()
        .filter(|app| {
            let name = app.name.to_lowercase();
            ![
                "uninstall",
                "readme",
                "documentation",
                "release notes",
                "website",
            ]
            .iter()
            .any(|word| name.contains(word) && !query.contains(word))
        })
        .collect::<Vec<_>>();
    let shortest = |matches: &dyn Fn(&str) -> bool| {
        candidates
            .iter()
            .filter(|app| matches(&app.name.to_lowercase()))
            .min_by_key(|app| app.name.len())
            .copied()
    };
    shortest(&|name| name == query)
        .or_else(|| shortest(&|name| name.starts_with(query)))
        .or_else(|| shortest(&|name| name.contains(query)))
}

#[cfg(test)]
mod tests {
    use super::{App, pick};

    fn apps() -> Vec<App> {
        [
            ("Claude", "com.squirrel.AnthropicClaude.claude"),
            ("Cursor", "Anysphere.Cursor"),
            ("Uninstall Cursor", "Anysphere.Cursor.Uninstall"),
            ("Notepad", "Microsoft.WindowsNotepad_8wekyb3d8bbwe!App"),
            ("Notepad++", "notepad++.exe"),
            ("Visual Studio Code", "Microsoft.VisualStudioCode"),
        ]
        .into_iter()
        .map(|(name, id)| App {
            name: name.into(),
            id: id.into(),
        })
        .collect()
    }

    #[test]
    fn names_resolve_to_start_menu_apps() {
        let apps = apps();
        let id = |query: &str| pick(&apps, query).map(|app| app.id.as_str());
        assert_eq!(id("cursor"), Some("Anysphere.Cursor"));
        assert_eq!(id("Claude"), Some("com.squirrel.AnthropicClaude.claude"));
        assert_eq!(
            id("notepad.exe"),
            Some("Microsoft.WindowsNotepad_8wekyb3d8bbwe!App")
        );
        assert_eq!(id("visual studio"), Some("Microsoft.VisualStudioCode"));
        assert_eq!(id("code"), Some("Microsoft.VisualStudioCode"));
        assert_eq!(id("uninstall cursor"), Some("Anysphere.Cursor.Uninstall"));
        assert_eq!(id("https://example.com"), None);
        assert_eq!(id(r"C:\tools\app.exe"), None);
        assert_eq!(id("shell:AppsFolder\\Anysphere.Cursor"), None);
        assert_eq!(id("zzz"), None);
    }
}
