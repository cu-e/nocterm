//! Conversion into component colour names and an opaque terminal palette.
use super::ZedTheme;
use nocterm_design::{Color, Palette};
use std::collections::BTreeMap;

impl ZedTheme {
    /// Refine missing ANSI colours from the supplied appearance palette.
    /// Its background is the opaque interface baseline used for compositing;
    /// the UI supplies the component default rather than theme.toml colours.
    pub fn palette(&self, base: &Palette) -> Palette {
        let get = |keys: &[&str]| keys.iter().find_map(|key| self.colors.get(*key).copied());
        let player = self.players.first();
        let cursor = player.and_then(|p| p.cursor);
        let selection = player.and_then(|p| p.selection);
        let mut ui = BTreeMap::new();
        let mut map = |targets: &[&str], keys: &[&str]| {
            if let Some(color) = get(keys) {
                for target in targets {
                    ui.insert((*target).to_owned(), color);
                }
            }
        };
        map(&["background"], &["editor.background", "background"]);
        map(&["foreground"], &["text", "editor.foreground"]);
        map(
            &[
                "border",
                "window.border",
                "sidebar.border",
                "title_bar.border",
                "status_bar.border",
            ],
            &["border"],
        );
        map(&["input.border"], &["border.variant", "border"]);
        map(&["ring", "list.active.border"], &["border.focused"]);
        map(
            &["muted.background"],
            &["element.background", "surface.background"],
        );
        map(&["muted.foreground", "tab.foreground"], &["text.muted"]);
        map(
            &["secondary.background", "tab_bar.segmented.background"],
            &["element.background"],
        );
        map(&["secondary.hover.background"], &["element.hover"]);
        map(&["secondary.active.background"], &["element.active"]);
        map(
            &[
                "secondary.foreground",
                "popover.foreground",
                "sidebar.foreground",
                "accent.foreground",
                "tab.active.foreground",
            ],
            &["text"],
        );
        map(
            &[
                "accent.background",
                "list.hover.background",
                "sidebar.accent.background",
            ],
            &["ghost_element.hover", "element.hover"],
        );
        map(
            &["list.active.background", "sidebar.primary.background"],
            &["ghost_element.selected", "element.selected"],
        );
        map(&["popover.background"], &["elevated_surface.background"]);
        map(
            &["sidebar.background", "list.head.background"],
            &["panel.background", "surface.background"],
        );
        map(
            &["title_bar.background"],
            &["title_bar.background", "background"],
        );
        map(
            &["status_bar.background"],
            &["status_bar.background", "background"],
        );
        map(&["tab_bar.background"], &["tab_bar.background"]);
        map(&["tab.background"], &["tab.inactive_background"]);
        map(&["tab.active.background"], &["tab.active_background"]);
        map(&["scrollbar.background"], &["scrollbar.track.background"]);
        map(
            &["scrollbar.thumb.background"],
            &["scrollbar.thumb.background"],
        );
        map(
            &["scrollbar.thumb.hover.background"],
            &["scrollbar.thumb.hover_background"],
        );
        map(&["link"], &["text.accent"]);
        map(&["link.hover"], &["link_text.hover"]);
        map(&["drop_target.background"], &["drop_target.background"]);
        for (target, source) in [
            ("danger", "error"),
            ("success", "success"),
            ("warning", "warning"),
            ("info", "info"),
        ] {
            map(&[&format!("{target}.background")], &[source]);
        }
        for (target, fallback) in [
            ("red", "error"),
            ("green", "success"),
            ("yellow", "warning"),
            ("blue", "info"),
            ("magenta", ""),
            ("cyan", ""),
        ] {
            map(
                &[&format!("base.{target}")],
                &[&format!("terminal.ansi.{target}"), fallback],
            );
        }
        if let Some(c) = get(&["text.accent"])
            .or(cursor)
            .or(get(&["border.focused"]))
        {
            ui.insert("primary.background".into(), c);
            if let (Some(background), Some(text)) = (
                get(&["editor.background", "background"]),
                get(&["text", "editor.foreground"]),
            ) {
                ui.insert(
                    "primary.foreground".into(),
                    if contrast(c, background) > contrast(c, text) {
                        background
                    } else {
                        text
                    },
                );
            }
        }
        if let Some(c) = cursor.or(get(&["text.accent", "border.focused"])) {
            ui.insert("caret".into(), c);
        }
        if let Some(c) = selection.or(get(&["element.selection_background"])) {
            ui.insert("selection.background".into(), c);
        }
        let base_background = base
            .terminal
            .background
            .or_else(|| {
                base.ui
                    .iter()
                    .find(|(key, _)| *key == "background")
                    .map(|(_, c)| c)
            })
            .unwrap_or(Color::rgb(0, 0, 0));
        let interface = opaque(
            get(&["editor.background", "background"]).unwrap_or(base_background),
            base_background,
        );
        let background = get(&["terminal.background", "terminal.ansi.background"]);
        let resolved = opaque(background.unwrap_or(interface), interface);
        let mut terminal = base.terminal.clone();
        terminal.background = background.map(|c| opaque(c, interface)).or_else(|| {
            get(&["editor.background", "background"])
                .filter(|c| c.a < 255)
                .map(|_| resolved)
        });
        terminal.foreground = get(&["terminal.foreground", "terminal.ansi.foreground", "text"])
            .map(|c| opaque(c, resolved))
            .or_else(|| {
                get(&["editor.foreground"])
                    .filter(|c| c.a < 255)
                    .map(|c| opaque(c, resolved))
            });
        terminal.cursor = cursor.map(|c| opaque(c, resolved));
        terminal.selection = selection;
        macro_rules! ansi { ($($key:ident),*) => { $(terminal.$key = opaque(get(&[concat!("terminal.ansi.", stringify!($key))]).unwrap_or(base.terminal.$key), resolved);)* }; }
        ansi!(
            black,
            red,
            green,
            yellow,
            blue,
            magenta,
            cyan,
            white,
            bright_black,
            bright_red,
            bright_green,
            bright_yellow,
            bright_blue,
            bright_magenta,
            bright_cyan,
            bright_white
        );
        Palette {
            ui: ui.into_iter().collect(),
            terminal,
        }
    }
}
fn opaque(color: Color, background: Color) -> Color {
    let mix = |c: u8, b: u8| {
        ((u32::from(c) * u32::from(color.a) + u32::from(b) * (255 - u32::from(color.a)) + 127)
            / 255) as u8
    };
    Color::rgb(
        mix(color.r, background.r),
        mix(color.g, background.g),
        mix(color.b, background.b),
    )
}
fn luminance(color: Color) -> f64 {
    let linear = |v: u8| {
        let v = f64::from(v) / 255.;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
}
fn contrast(a: Color, b: Color) -> f64 {
    let a = luminance(a);
    let b = luminance(b);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
