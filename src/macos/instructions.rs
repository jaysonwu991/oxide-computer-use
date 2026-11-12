//! Independently authored guidance for apps whose accessibility UI benefits
//! from a short, task-specific orientation.

const SLACK: &str = "\
Slack:
- Use set_value on the message composer when you need a multiline message.
- A Return key in the composer may submit the message. Prefer paste for multiline content.
- Use list_apps/query to distinguish the composer from search and thread editors.";

const NOTION: &str = "\
Notion:
- Prefer clicking a block before typing so keyboard events reach the page editor.
- Use query to locate the current page editor and toolbar in a large document tree.
- Re-read state after opening menus or switching pages; their accessibility subtrees are transient.";

const SPOTIFY: &str = "\
Spotify:
- Use search for navigation and then operate result rows by element index.
- Prefer AXPress for play/pause and result actions when available.
- Re-read state after navigation because the now-playing and list panes update independently.";

const APPLE_MUSIC: &str = "\
Apple Music:
- Use search to navigate to songs, albums, or playlists before acting.
- Prefer indexed buttons for play controls; coordinate clicks can miss controls that move with the player.
- Re-read state after playback changes because several panes update at once.";

const NUMBERS: &str = "\
Numbers:
- Enter a cell or contiguous row at a time; avoid submitting multiple separated rows in one type_text call.
- Use Tab between cells and Return to commit a cell edit.
- Prefer set_value when a cell exposes a settable AXValue, then verify the displayed value.";

const CLOCK: &str = "\
Clock:
- Use the on-screen labels to distinguish start, pause, and reset controls before clicking.
- Prefer set_value for editable duration fields when available.
- Re-read state after starting a timer because the control labels and values change.";

const IPHONE_MIRRORING: &str = "\
iPhone Mirroring:
- Treat the mirrored phone as a separate GUI and use indexed elements when available.
- Pointer coordinates are relative to the iPhone Mirroring window screenshot.
- Re-read state after navigation, rotation, keyboard changes, or interruptions.";

const APPS: &[(&str, &str, &str)] = &[
    ("com.tinyspeck.slackmacgap", "Slack", SLACK),
    ("notion.id", "Notion", NOTION),
    ("com.spotify.client", "Spotify", SPOTIFY),
    ("com.apple.Music", "Music", APPLE_MUSIC),
    ("com.apple.iWork.Numbers", "Numbers", NUMBERS),
    ("com.apple.clock", "Clock", CLOCK),
    (
        "com.apple.ScreenContinuity",
        "iPhone Mirroring",
        IPHONE_MIRRORING,
    ),
];

pub fn for_app(bundle_id: Option<&str>, display_name: &str) -> Option<&'static str> {
    APPS.iter()
        .find(|(bundle, name, _)| {
            bundle_id.is_some_and(|bundle_id| bundle.eq_ignore_ascii_case(bundle_id))
                || name.eq_ignore_ascii_case(display_name)
        })
        .map(|(_, _, instructions)| *instructions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_apps_get_their_own_guidance() {
        assert_eq!(
            for_app(Some("com.tinyspeck.slackmacgap"), "ignored"),
            Some(SLACK)
        );
        assert_eq!(for_app(None, "Numbers"), Some(NUMBERS));
        assert_eq!(for_app(Some("com.example.other"), "Other"), None);
    }

    #[test]
    fn guidance_is_short_and_actionable() {
        for (_, _, instructions) in APPS {
            assert!(instructions.chars().count() < 500, "{instructions}");
            assert!(instructions.contains(':'), "{instructions}");
        }
    }
}
