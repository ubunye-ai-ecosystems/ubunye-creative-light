//! Community and project links (Help menu, About dialog, top-bar questions button).

/// The app's id in product-page URLs.
pub const APP: &str = "light";
/// Questions and ideas (the issue tracker; the command id stays `app.discord`).
pub const DISCORD: &str = lightcraft_engine::brand::ISSUES;
/// The suite's website.
pub const WEBSITE: &str = lightcraft_engine::brand::SUITE_URL;
/// This app's product page.
pub const APP_PAGE: &str = lightcraft_engine::brand::HOMEPAGE;
/// This app's source repository.
pub const GITHUB: &str = lightcraft_engine::brand::SOURCE;

/// (UI command id, menu label, URL) for each link, in Help-menu order.
/// The user documentation (docs/ in the repository).
pub const HELP: &str = "https://github.com/ubunye-ai-ecosystems/ubunye-creative-light/tree/main/docs";

pub const LINKS: &[(&str, &str, &str)] = &[
    ("app.help", "Ubunye Light Help", HELP),
    ("app.discord", "Questions and Ideas…", DISCORD),
    ("app.website", "Ubunye Light Website", APP_PAGE),
    ("app.github", "Ubunye Light on GitHub", GITHUB),
    ("app.artcraft", "Ubunye Creative Studio Website", WEBSITE),
    ("app.feedback", "Send Feedback…", FEEDBACK),
];

/// Where feedback and bug reports go.
pub const FEEDBACK: &str = "https://github.com/ubunye-ai-ecosystems/ubunye-creative-light/issues/new";

/// The URL behind a link command id.
pub fn url_of(cmd: &str) -> Option<&'static str> {
    LINKS.iter().find(|(id, _, _)| *id == cmd).map(|(_, _, u)| *u)
}

/// Open `url` in the user's browser (through the host's `open_url` service).
pub fn open(app: &mut crate::LightcraftApp, url: &str) -> Result<serde_json::Value, String> {
    let open = app.services.open_url.as_mut().ok_or("can't open links here")?;
    open(url)?;
    Ok(serde_json::json!({ "url": url }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_are_https_and_name_this_app() {
        for (id, label, url) in LINKS {
            assert!(url.starts_with("https://"), "{id}");
            assert!(!label.is_empty());
            assert_eq!(url_of(id), Some(*url));
        }
        assert!(APP_PAGE.ends_with(&format!("/tools/{APP}")));
        assert!(GITHUB.ends_with("/ubunye-ai-ecosystems/ubunye-creative-light"));
    }
}
