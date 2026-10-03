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
        ArrowDown,
        // The AI agent panel.
        Bot,
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
    ]
);

/// The application's asset source: nocterm's icons over the component
/// library's defaults.
#[derive(Clone, Copy, Debug, Default)]
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match ExtraIcons.load(path)? {
            Some(data) => Ok(Some(data)),
            None => gpui_kit::assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = ExtraIcons.list(path)?;
        paths.extend(gpui_kit::assets::Assets.list(path)?);
        Ok(paths)
    }
}
