//! The icons nocterm uses, embedded in the binary.
//!
//! The component library bundles a small default set. Every further icon a
//! view uses is listed here once; an icon missing from the list renders
//! blank, so the list is also the inventory.

use std::borrow::Cow;

pub use gpui_kit::assets::IconName;
use gpui_kit::{AssetSource, Result, SharedString};

gpui_kit::assets::icon_assets!(
    ExtraIcons,
    [
        Cable,
        Clock,
        FolderTree,
        KeyRound,
        Pencil,
        Plug,
        Server,
        ServerOff,
        ServerPlus,
        Trash,
        Unplug,
        X,
        Zap,
        Upload,
        RefreshCw,
        ArrowUp,
        ArrowUpRight,
        ChevronsUp,
        ArrowDown,
        // The AI agent panel.
        Bot,
        Terminal,
        SquareTerminal,
        Palette,
        Lock,
        FingerprintPattern,
        Sparkles,
        Maximize2,
        Minimize2,
        Star,
        StarFill,
        Paperclip,
        ImagePlus,
        CircleStop,
        Brain,
        Gauge,
        MessageSquarePlus,
        PanelRight,
        ShieldCheck,
        Wrench,
        ListTodo,
        LoaderCircle,
        Layers,
        Send,
        // The keymap page.
        Keyboard,
        RotateCcw,
        // Chat history actions.
        Pin,
        PinOff,
        GitFork,
        // The host monitor.
        Activity,
        Thermometer,
        // The containers panel.
        Container,
        Boxes,
        Image,
        Square,
        ScrollText,
        Play,
        // Icons only a user plugin contribution may show (`Icon::user_allowed`).
        Box,
        Gem,
        Hexagon,
        Puzzle,
        Shapes,
    ]
);

/// Where nocterm's own mark, a monochrome glyph tinted by its text colour, is
/// served. It is twice as wide as it is tall.
pub const APP_MARK: &str = "icons/brand/nocterm.svg";
const APP_MARK_SVG: &[u8] = include_bytes!("../assets/icons/nocterm.svg");

/// The application's asset source: nocterm's mark, agent brand marks and its icons
/// over the component library's defaults.
#[derive(Clone, Debug, Default)]
pub struct Assets {
    marks: Vec<(&'static str, &'static [u8])>,
}

impl Assets {
    /// Also serve each `(path, svg)` brand mark, such as the built-in agents'.
    pub fn with_marks(marks: impl IntoIterator<Item = (&'static str, &'static [u8])>) -> Self {
        Self {
            marks: marks.into_iter().collect(),
        }
    }
}

/// An agent's icon: its brand mark served at `mark`, in the `0xRRGGBB` `tint`
/// when it has one, or a generic bot for agents without a mark.
pub fn agent_icon(mark: Option<&'static str>, tint: Option<u32>) -> gpui_kit::component::Icon {
    use gpui_kit::{Styled as _, component::Icon};
    let Some(mark) = mark else {
        return Icon::new(IconName::Bot);
    };
    let icon = Icon::empty().path(mark);
    match tint {
        Some(rgb) => icon.text_color(gpui_kit::rgb(rgb)),
        None => icon,
    }
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == APP_MARK {
            return Ok(Some(Cow::Borrowed(APP_MARK_SVG)));
        }
        if let Some((_, data)) = self.marks.iter().find(|(name, _)| *name == path) {
            return Ok(Some(Cow::Borrowed(data)));
        }
        match ExtraIcons.load(path)? {
            Some(data) => Ok(Some(data)),
            None => gpui_kit::assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths: Vec<SharedString> = std::iter::once(APP_MARK)
            .chain(self.marks.iter().map(|(name, _)| *name))
            .filter(|name| name.starts_with(path))
            .map(Into::into)
            .collect();
        paths.extend(ExtraIcons.list(path)?);
        paths.extend(gpui_kit::assets::Assets.list(path)?);
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_mark_is_served_and_listed() {
        let assets = Assets::default();
        let mark = assets
            .load(APP_MARK)
            .unwrap()
            .expect("the mark is embedded");
        assert!(std::str::from_utf8(&mark).unwrap().starts_with("<svg"));
        assert!(
            assets
                .list("icons/brand/")
                .unwrap()
                .iter()
                .any(|path| path == APP_MARK)
        );
    }
}
