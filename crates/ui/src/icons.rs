//! The icons nocterm uses, embedded in the binary.
//!
//! The component library bundles a small default set. Every further icon a
//! view uses is listed here once; an icon missing from the list renders
//! blank, so the list is also the inventory.

use std::borrow::Cow;

pub use gpui_kit::assets::IconName;
use gpui_kit::{AssetSource, Result, SharedString, Styled as _};

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
        ArrowDown,
        // The AI agent panel.
        Bot,
        Terminal,
        SquareTerminal,
        Palette,
        Lock,
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
    ]
);

/// The application's asset source: nocterm's icons over the component
/// library's defaults.
#[derive(Clone, Copy, Debug, Default)]
pub struct Assets;

/// Brand marks of the built-in agents, drawn in one colour like every icon.
const AGENT_ICONS: &[(&str, &[u8])] = &[
    (
        "icons/agents/claude.svg",
        include_bytes!("../assets/agents/claude.svg"),
    ),
    (
        "icons/agents/codex.svg",
        include_bytes!("../assets/agents/codex.svg"),
    ),
    (
        "icons/agents/hermes.svg",
        include_bytes!("../assets/agents/hermes.svg"),
    ),
];

/// The icon of agent `id`: its brand mark and colour when nocterm knows
/// the agent, a generic bot otherwise.
pub fn agent_icon(id: &str) -> gpui_kit::component::Icon {
    use gpui_kit::component::Icon;
    let mark = |path: &'static str| Icon::empty().path(path);
    match id {
        "claude" => mark("icons/agents/claude.svg").text_color(gpui_kit::rgb(0xd97757)),
        "codex" => mark("icons/agents/codex.svg"),
        "hermes" => mark("icons/agents/hermes.svg"),
        _ => Icon::new(IconName::Bot),
    }
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, data)) = AGENT_ICONS.iter().find(|(name, _)| *name == path) {
            return Ok(Some(Cow::Borrowed(data)));
        }
        match ExtraIcons.load(path)? {
            Some(data) => Ok(Some(data)),
            None => gpui_kit::assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths: Vec<SharedString> = AGENT_ICONS
            .iter()
            .map(|(name, _)| *name)
            .filter(|name| name.starts_with(path))
            .map(Into::into)
            .collect();
        paths.extend(ExtraIcons.list(path)?);
        paths.extend(gpui_kit::assets::Assets.list(path)?);
        Ok(paths)
    }
}
