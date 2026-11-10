//! The icons a plugin names: [`PluginIcon`] for its contributions, [`Glyph`] inside its views.

use nocterm_plugin_model::Icon;
use serde::{Deserialize, Serialize};

/// An icon a plugin names. Names are frozen and the list is append-only.
///
/// Each is an icon that no system plugin or core contribution draws, so a plugin toggle cannot
/// imitate a host function (`Icon::user_allowed`; the app checks the catalog against this set).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum PluginIcon {
    Box,
    Gem,
    Hexagon,
    Plug,
    Puzzle,
    Shapes,
}

impl PluginIcon {
    /// Every icon, in declaration order.
    pub const ALL: [Self; 6] = [
        Self::Box,
        Self::Gem,
        Self::Hexagon,
        Self::Plug,
        Self::Puzzle,
        Self::Shapes,
    ];

    /// The frozen kebab-case name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Box => "box",
            Self::Gem => "gem",
            Self::Hexagon => "hexagon",
            Self::Plug => "plug",
            Self::Puzzle => "puzzle",
            Self::Shapes => "shapes",
        }
    }

    /// The host icon this one draws as.
    pub const fn to_icon(self) -> Icon {
        match self {
            Self::Box => Icon::Box,
            Self::Gem => Icon::Gem,
            Self::Hexagon => Icon::Hexagon,
            Self::Plug => Icon::Plug,
            Self::Puzzle => Icon::Puzzle,
            Self::Shapes => Icon::Shapes,
        }
    }
}

/// An icon inside a plugin's own content: a row's leading icon, an icon button, an icon node.
///
/// A plugin may draw any of these in the frame the host gives it, where none can stand for a
/// function of the application; what a plugin shows outside that frame (the switcher of its
/// panel) is a [`PluginIcon`]. Names are frozen and the list is append-only; the first six are
/// the plugin icons, in the same order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Glyph {
    Box,
    Gem,
    Hexagon,
    Plug,
    Puzzle,
    Shapes,
    Activity,
    ArrowDown,
    ArrowLeft,
    ArrowUp,
    Bell,
    Cable,
    Calendar,
    Check,
    ChevronDown,
    ChevronLeft,
    ChevronRight,
    CircleAlert,
    CircleCheck,
    CircleX,
    Clock,
    Container,
    Copy,
    Cpu,
    Ellipsis,
    ExternalLink,
    Eye,
    File,
    FileText,
    Folder,
    FolderOpen,
    Gauge,
    Globe,
    HardDrive,
    Inbox,
    Info,
    Key,
    Loader,
    Lock,
    Memory,
    Network,
    Pause,
    Pencil,
    Play,
    Plus,
    Minus,
    Refresh,
    Rotate,
    Search,
    Send,
    Server,
    ServerOff,
    Settings,
    Square,
    Stop,
    Star,
    Terminal,
    Thermometer,
    Trash,
    TriangleAlert,
    Upload,
    User,
    Wrench,
    X,
    Zap,
}

impl Glyph {
    /// Every glyph, in declaration order.
    pub const ALL: [Self; 65] = [
        Self::Box,
        Self::Gem,
        Self::Hexagon,
        Self::Plug,
        Self::Puzzle,
        Self::Shapes,
        Self::Activity,
        Self::ArrowDown,
        Self::ArrowLeft,
        Self::ArrowUp,
        Self::Bell,
        Self::Cable,
        Self::Calendar,
        Self::Check,
        Self::ChevronDown,
        Self::ChevronLeft,
        Self::ChevronRight,
        Self::CircleAlert,
        Self::CircleCheck,
        Self::CircleX,
        Self::Clock,
        Self::Container,
        Self::Copy,
        Self::Cpu,
        Self::Ellipsis,
        Self::ExternalLink,
        Self::Eye,
        Self::File,
        Self::FileText,
        Self::Folder,
        Self::FolderOpen,
        Self::Gauge,
        Self::Globe,
        Self::HardDrive,
        Self::Inbox,
        Self::Info,
        Self::Key,
        Self::Loader,
        Self::Lock,
        Self::Memory,
        Self::Network,
        Self::Pause,
        Self::Pencil,
        Self::Play,
        Self::Plus,
        Self::Minus,
        Self::Refresh,
        Self::Rotate,
        Self::Search,
        Self::Send,
        Self::Server,
        Self::ServerOff,
        Self::Settings,
        Self::Square,
        Self::Stop,
        Self::Star,
        Self::Terminal,
        Self::Thermometer,
        Self::Trash,
        Self::TriangleAlert,
        Self::Upload,
        Self::User,
        Self::Wrench,
        Self::X,
        Self::Zap,
    ];

    /// The frozen kebab-case name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Box => "box",
            Self::Gem => "gem",
            Self::Hexagon => "hexagon",
            Self::Plug => "plug",
            Self::Puzzle => "puzzle",
            Self::Shapes => "shapes",
            Self::Activity => "activity",
            Self::ArrowDown => "arrow-down",
            Self::ArrowLeft => "arrow-left",
            Self::ArrowUp => "arrow-up",
            Self::Bell => "bell",
            Self::Cable => "cable",
            Self::Calendar => "calendar",
            Self::Check => "check",
            Self::ChevronDown => "chevron-down",
            Self::ChevronLeft => "chevron-left",
            Self::ChevronRight => "chevron-right",
            Self::CircleAlert => "circle-alert",
            Self::CircleCheck => "circle-check",
            Self::CircleX => "circle-x",
            Self::Clock => "clock",
            Self::Container => "container",
            Self::Copy => "copy",
            Self::Cpu => "cpu",
            Self::Ellipsis => "ellipsis",
            Self::ExternalLink => "external-link",
            Self::Eye => "eye",
            Self::File => "file",
            Self::FileText => "file-text",
            Self::Folder => "folder",
            Self::FolderOpen => "folder-open",
            Self::Gauge => "gauge",
            Self::Globe => "globe",
            Self::HardDrive => "hard-drive",
            Self::Inbox => "inbox",
            Self::Info => "info",
            Self::Key => "key",
            Self::Loader => "loader",
            Self::Lock => "lock",
            Self::Memory => "memory",
            Self::Network => "network",
            Self::Pause => "pause",
            Self::Pencil => "pencil",
            Self::Play => "play",
            Self::Plus => "plus",
            Self::Minus => "minus",
            Self::Refresh => "refresh",
            Self::Rotate => "rotate",
            Self::Search => "search",
            Self::Send => "send",
            Self::Server => "server",
            Self::ServerOff => "server-off",
            Self::Settings => "settings",
            Self::Square => "square",
            Self::Stop => "stop",
            Self::Star => "star",
            Self::Terminal => "terminal",
            Self::Thermometer => "thermometer",
            Self::Trash => "trash",
            Self::TriangleAlert => "triangle-alert",
            Self::Upload => "upload",
            Self::User => "user",
            Self::Wrench => "wrench",
            Self::X => "x",
            Self::Zap => "zap",
        }
    }
}

impl From<PluginIcon> for Glyph {
    fn from(icon: PluginIcon) -> Self {
        match icon {
            PluginIcon::Box => Self::Box,
            PluginIcon::Gem => Self::Gem,
            PluginIcon::Hexagon => Self::Hexagon,
            PluginIcon::Plug => Self::Plug,
            PluginIcon::Puzzle => Self::Puzzle,
            PluginIcon::Shapes => Self::Shapes,
        }
    }
}
