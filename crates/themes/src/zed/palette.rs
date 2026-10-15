//! Conversion into component colour names and an opaque terminal palette.
use super::ZedTheme;
use nocterm_design::{Color, Palette};
use std::collections::BTreeMap;

impl ZedTheme {
    /// Refine missing ANSI colours from the supplied appearance palette.
    /// Its background is the opaque interface baseline used for compositing;
    /// the UI supplies the component default rather than theme.toml colours.
    #[expect(clippy::too_many_lines, reason = "predates the limit")]
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
        terminal.normal.black = opaque(
            get(&["terminal.ansi.black"]).unwrap_or(base.terminal.normal.black),
            resolved,
        );
        terminal.normal.red = opaque(
            get(&["terminal.ansi.red"]).unwrap_or(base.terminal.normal.red),
            resolved,
        );
        terminal.normal.green = opaque(
            get(&["terminal.ansi.green"]).unwrap_or(base.terminal.normal.green),
            resolved,
        );
        terminal.normal.yellow = opaque(
            get(&["terminal.ansi.yellow"]).unwrap_or(base.terminal.normal.yellow),
            resolved,
        );
        terminal.normal.blue = opaque(
            get(&["terminal.ansi.blue"]).unwrap_or(base.terminal.normal.blue),
            resolved,
        );
        terminal.normal.magenta = opaque(
            get(&["terminal.ansi.magenta"]).unwrap_or(base.terminal.normal.magenta),
            resolved,
        );
        terminal.normal.cyan = opaque(
            get(&["terminal.ansi.cyan"]).unwrap_or(base.terminal.normal.cyan),
            resolved,
        );
        terminal.normal.white = opaque(
            get(&["terminal.ansi.white"]).unwrap_or(base.terminal.normal.white),
            resolved,
        );
        terminal.bright.black = opaque(
            get(&["terminal.ansi.bright_black"]).unwrap_or(base.terminal.bright.black),
            resolved,
        );
        terminal.bright.red = opaque(
            get(&["terminal.ansi.bright_red"]).unwrap_or(base.terminal.bright.red),
            resolved,
        );
        terminal.bright.green = opaque(
            get(&["terminal.ansi.bright_green"]).unwrap_or(base.terminal.bright.green),
            resolved,
        );
        terminal.bright.yellow = opaque(
            get(&["terminal.ansi.bright_yellow"]).unwrap_or(base.terminal.bright.yellow),
            resolved,
        );
        terminal.bright.blue = opaque(
            get(&["terminal.ansi.bright_blue"]).unwrap_or(base.terminal.bright.blue),
            resolved,
        );
        terminal.bright.magenta = opaque(
            get(&["terminal.ansi.bright_magenta"]).unwrap_or(base.terminal.bright.magenta),
            resolved,
        );
        terminal.bright.cyan = opaque(
            get(&["terminal.ansi.bright_cyan"]).unwrap_or(base.terminal.bright.cyan),
            resolved,
        );
        terminal.bright.white = opaque(
            get(&["terminal.ansi.bright_white"]).unwrap_or(base.terminal.bright.white),
            resolved,
        );
        // Zed's own `background` is the workspace behind its panels and
        // editors, which is what floating cards sit on. Themes that leave it
        // equal to the editor have no backdrop of their own; one is derived.
        let canvas = get(&["background"])
            .filter(|canvas| get(&["editor.background"]).is_some_and(|editor| editor != *canvas))
            .map(|canvas| opaque(canvas, interface));
        Palette {
            ui: ui.into_iter().collect(),
            canvas,
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
