//! What a plugin draws: a bounded arena of nodes.
//!
//! A view is plain data. The host sanitizes every string, validates the arena, and draws it
//! inside a frame it controls. There is no input, image, link, URL or drag node; a code block
//! shows lines of text and nothing it holds is ever run or followed.
//!
//! Besides the layout primitives (row, column, list, text, …) a view is built from the
//! components the application's own panels are made of: a [`NodeKind::Section`] with its
//! heading, an [`NodeKind::Item`] row with an icon, a title and a detail line, a small
//! [`NodeKind::IconButton`], a [`NodeKind::Caption`], a [`NodeKind::Field`] and a
//! [`NodeKind::Chart`] of recent values. The host draws them exactly as it draws its own, so a
//! plugin looks like part of the application without styling anything.

use alloc::{collections::BTreeSet, string::String, vec, vec::Vec};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{Glyph, LimitError, error::key_syntax, sanitize::sanitize_code, sanitize_display};

/// Most nodes in one view.
pub const MAX_NODES: usize = 256;
/// Deepest nesting, counting the root as 1.
pub const MAX_DEPTH: u8 = 8;
/// Longest text node, in characters.
pub const MAX_TEXT: usize = 200;
/// Longest badge, in characters.
pub const MAX_BADGE: usize = 8;
/// Longest button label, in characters.
pub const MAX_BUTTON: usize = 48;
/// Most children of a list.
pub const MAX_LIST_ITEMS: usize = 100;
/// Longest node key.
pub const MAX_NODE_KEY: usize = 32;
/// Longest code block, in characters, line breaks included.
pub const MAX_CODE: usize = 16 * 1024;
/// Shortest refresh delay a view may ask for.
pub const MIN_REFRESH_MS: u32 = 250;
/// Most values in one series of a chart, and the most a chart's width holds.
pub const MAX_CHART_POINTS: usize = 120;
/// Most series one chart draws over each other.
pub const MAX_CHART_SERIES: usize = 2;

/// How a node is coloured. The application takes every tone from its theme.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Tone {
    #[default]
    Default,
    Muted,
    Success,
    Caution,
}

impl Tone {
    /// Every tone, in declaration order.
    pub const ALL: [Self; 4] = [Self::Default, Self::Muted, Self::Success, Self::Caution];

    /// The frozen kebab-case name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Muted => "muted",
            Self::Success => "success",
            Self::Caution => "caution",
        }
    }
}

/// When a view wants to be drawn again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Refresh {
    NextSecond,
    NextMinute,
    /// After this many milliseconds, at least [`MIN_REFRESH_MS`].
    AfterMs(u32),
}

/// What a node is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NodeKind {
    Row,
    Column,
    Text(String),
    Icon(Glyph),
    /// A bar from 0 to 100.
    Meter(u8),
    Badge(String),
    /// Needs a key, which the `activated` event carries.
    Button(String),
    /// Needs a key, which the `toggled` event carries.
    Toggle(bool),
    /// A host-scrolled run of children.
    List,
    Divider,
    Spacer,
    /// Lines of text in a monospace block the host scrolls, such as a file's contents.
    Code(String),
    /// A titled group of children, headed as the application heads its own: the title in small
    /// capitals with an optional count at the end. With a key the user can fold it; the host
    /// keeps whether it is folded, and `folded` is how it starts.
    Section {
        title: String,
        count: Option<String>,
        folded: bool,
    },
    /// A row of a list, drawn as the application draws its own: an optional icon in the node's
    /// tone, a title and an optional muted detail line, then its children (icon buttons,
    /// badges) at the end. With a key the whole row reports `activated`; `selected` marks the
    /// current one.
    Item {
        title: String,
        detail: Option<String>,
        icon: Option<Glyph>,
        selected: bool,
    },
    /// A small borderless button that shows an icon and names its action in a tooltip. Needs a
    /// key, which the `activated` event carries.
    IconButton {
        icon: Glyph,
        label: String,
    },
    /// A line of small muted text: a hint, an empty list, a state.
    Caption(String),
    /// A label and its value on one line, such as one fact of the selected thing; the tone
    /// colours the value.
    Field {
        label: String,
        value: String,
    },
    /// Recent values as a small area chart, drawn as the application draws its own graphs:
    /// each series a line over a light fill, from 0 at the bottom to `max` at the top (the
    /// largest value when `None`), the newest value at the right end. The width holds `span`
    /// values (all of them when `None`), so a short history grows in from the right. One or two
    /// series of at most 120 values; the first takes the node's tone, the second the
    /// application's second chart colour.
    Chart {
        series: Vec<Vec<u32>>,
        max: Option<u32>,
        span: Option<u16>,
    },
}

impl NodeKind {
    /// The frozen kebab-case name.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Row => "row",
            Self::Column => "column",
            Self::Text(_) => "text",
            Self::Icon(_) => "icon",
            Self::Meter(_) => "meter",
            Self::Badge(_) => "badge",
            Self::Button(_) => "button",
            Self::Toggle(_) => "toggle",
            Self::List => "list",
            Self::Divider => "divider",
            Self::Spacer => "spacer",
            Self::Code(_) => "code",
            Self::Section { .. } => "section",
            Self::Item { .. } => "item",
            Self::IconButton { .. } => "icon-button",
            Self::Caption(_) => "caption",
            Self::Field { .. } => "field",
            Self::Chart { .. } => "chart",
        }
    }

    /// The names of all kinds, for the API surface.
    pub const NAMES: [&'static str; 18] = [
        "row",
        "column",
        "text",
        "icon",
        "meter",
        "badge",
        "button",
        "toggle",
        "list",
        "divider",
        "spacer",
        "code",
        "section",
        "item",
        "icon-button",
        "caption",
        "field",
        "chart",
    ];

    fn holds_children(&self) -> bool {
        matches!(
            self,
            Self::Row | Self::Column | Self::List | Self::Section { .. } | Self::Item { .. }
        )
    }

    fn needs_key(&self) -> bool {
        matches!(
            self,
            Self::Button(_) | Self::Toggle(_) | Self::IconButton { .. }
        )
    }
}

/// One node of the arena.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Node {
    /// The index of the parent, which comes earlier in the arena. `None` for a root.
    #[serde(default)]
    pub parent: Option<u16>,
    #[serde(default)]
    pub key: Option<String>,
    pub kind: NodeKind,
    #[serde(default)]
    pub tone: Tone,
}

impl Node {
    pub fn new(kind: NodeKind) -> Self {
        Self {
            parent: None,
            key: None,
            kind,
            tone: Tone::Default,
        }
    }
}

/// A view a plugin returns.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct View {
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub refresh: Option<Refresh>,
}

/// A view the host refuses to draw. The host draws a fixed-width fault chip instead.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ViewError {
    #[error("a view has at most {MAX_NODES} nodes")]
    TooManyNodes,
    #[error("node {index} names a parent that does not come before it")]
    Parent { index: usize },
    #[error("node {index} has a parent that cannot hold children")]
    NotAContainer { index: usize },
    #[error("node {index} is nested deeper than {MAX_DEPTH} levels")]
    TooDeep { index: usize },
    #[error("a list has more than {MAX_LIST_ITEMS} items")]
    TooManyItems,
    #[error("node {index} needs a key")]
    MissingKey { index: usize },
    #[error("key `{key}` is used twice")]
    DuplicateKey { key: String },
    #[error("a refresh must wait at least {MIN_REFRESH_MS} ms")]
    Refresh,
    #[error("node {index}: {source}")]
    Limit {
        index: usize,
        #[source]
        source: LimitError,
    },
}

impl View {
    /// Sanitizes every string and cuts it to its limit. Run it on guest output before
    /// [`validate`](Self::validate); it is idempotent.
    pub fn sanitize(&mut self) {
        for node in &mut self.nodes {
            match &mut node.kind {
                NodeKind::Text(text) => *text = sanitize_display(text, MAX_TEXT),
                NodeKind::Badge(text) => *text = sanitize_display(text, MAX_BADGE),
                NodeKind::Button(text) => *text = sanitize_display(text, MAX_BUTTON),
                NodeKind::Code(text) => *text = sanitize_code(text, MAX_CODE),
                NodeKind::Caption(text) => *text = sanitize_display(text, MAX_TEXT),
                NodeKind::Section { title, count, .. } => {
                    *title = sanitize_display(title, MAX_BUTTON);
                    if let Some(count) = count {
                        *count = sanitize_display(count, MAX_BADGE);
                    }
                }
                NodeKind::Item { title, detail, .. } => {
                    *title = sanitize_display(title, MAX_TEXT);
                    if let Some(detail) = detail {
                        *detail = sanitize_display(detail, MAX_TEXT);
                    }
                }
                NodeKind::IconButton { label, .. } => *label = sanitize_display(label, MAX_BUTTON),
                NodeKind::Field { label, value } => {
                    *label = sanitize_display(label, MAX_BUTTON);
                    *value = sanitize_display(value, MAX_TEXT);
                }
                _ => {}
            }
        }
    }

    /// Sanitizes, then validates.
    pub fn into_validated(mut self) -> Result<Self, ViewError> {
        self.sanitize();
        self.validate()?;
        Ok(self)
    }

    /// Checks the arena: at most 256 nodes, every parent earlier and able to hold children,
    /// depth at most 8, unique keys, keys on buttons and toggles, list size and text limits.
    pub fn validate(&self) -> Result<(), ViewError> {
        if self.nodes.len() > MAX_NODES {
            return Err(ViewError::TooManyNodes);
        }
        if matches!(self.refresh, Some(Refresh::AfterMs(ms)) if ms < MIN_REFRESH_MS) {
            return Err(ViewError::Refresh);
        }
        let mut depth = vec![1u8; self.nodes.len()];
        let mut children = vec![0usize; self.nodes.len()];
        let mut keys = BTreeSet::new();
        for (index, node) in self.nodes.iter().enumerate() {
            if let Some(parent) = node.parent {
                let parent = usize::from(parent);
                self.check_parent(index, parent)?;
                depth[index] = depth[parent].saturating_add(1);
                children[parent] += 1;
                if depth[index] > MAX_DEPTH {
                    return Err(ViewError::TooDeep { index });
                }
                if self.nodes[parent].kind == NodeKind::List && children[parent] > MAX_LIST_ITEMS {
                    return Err(ViewError::TooManyItems);
                }
            }
            check_content(index, &node.kind)?;
            check_key(index, node, &mut keys)?;
        }
        Ok(())
    }

    fn check_parent(&self, index: usize, parent: usize) -> Result<(), ViewError> {
        if parent >= index {
            return Err(ViewError::Parent { index });
        }
        if !self.nodes[parent].kind.holds_children() {
            return Err(ViewError::NotAContainer { index });
        }
        Ok(())
    }
}

fn check_content(index: usize, kind: &NodeKind) -> Result<(), ViewError> {
    let limit = |source| ViewError::Limit { index, source };
    let bounded = |text: &str, max, field| -> Result<(), ViewError> {
        if text.chars().count() > max {
            return Err(limit(LimitError::TooLong { field, max }));
        }
        if sanitize_display(text, max) != text {
            return Err(limit(LimitError::Unsanitized { field }));
        }
        Ok(())
    };
    match kind {
        NodeKind::Text(text) => bounded(text, MAX_TEXT, "text"),
        NodeKind::Badge(text) => bounded(text, MAX_BADGE, "badge"),
        NodeKind::Button(text) => bounded(text, MAX_BUTTON, "button"),
        NodeKind::Code(text) => {
            if text.chars().count() > MAX_CODE {
                return Err(limit(LimitError::TooLong {
                    field: "code",
                    max: MAX_CODE,
                }));
            }
            if sanitize_code(text, MAX_CODE) != *text {
                return Err(limit(LimitError::Unsanitized { field: "code" }));
            }
            Ok(())
        }
        NodeKind::Meter(value) if *value > 100 => Err(limit(LimitError::Range {
            field: "meter",
            max: 100,
        })),
        NodeKind::Caption(text) => bounded(text, MAX_TEXT, "caption"),
        NodeKind::Section { title, count, .. } => {
            bounded(title, MAX_BUTTON, "section title")?;
            count
                .as_deref()
                .map_or(Ok(()), |count| bounded(count, MAX_BADGE, "section count"))
        }
        NodeKind::Item { title, detail, .. } => {
            bounded(title, MAX_TEXT, "item title")?;
            detail
                .as_deref()
                .map_or(Ok(()), |detail| bounded(detail, MAX_TEXT, "item detail"))
        }
        NodeKind::IconButton { label, .. } => bounded(label, MAX_BUTTON, "icon button label"),
        NodeKind::Field { label, value } => {
            bounded(label, MAX_BUTTON, "field label")?;
            bounded(value, MAX_TEXT, "field value")
        }
        NodeKind::Chart { series, span, .. } => check_chart(series, *span).map_err(limit),
        _ => Ok(()),
    }
}

/// One or two series of at most [`MAX_CHART_POINTS`] values, and a span within the same.
fn check_chart(series: &[Vec<u32>], span: Option<u16>) -> Result<(), LimitError> {
    if series.is_empty() {
        return Err(LimitError::Empty {
            field: "chart series",
        });
    }
    if series.len() > MAX_CHART_SERIES {
        return Err(LimitError::TooMany {
            field: "chart series",
            max: MAX_CHART_SERIES,
        });
    }
    if series.iter().any(|values| values.len() > MAX_CHART_POINTS) {
        return Err(LimitError::TooMany {
            field: "chart values",
            max: MAX_CHART_POINTS,
        });
    }
    match span {
        Some(span) if span == 0 || usize::from(span) > MAX_CHART_POINTS => Err(LimitError::Range {
            field: "chart span",
            max: MAX_CHART_POINTS as u32,
        }),
        _ => Ok(()),
    }
}

fn check_key<'a>(
    index: usize,
    node: &'a Node,
    keys: &mut BTreeSet<&'a str>,
) -> Result<(), ViewError> {
    match node.key.as_deref() {
        None if node.kind.needs_key() => Err(ViewError::MissingKey { index }),
        None => Ok(()),
        Some(key) => {
            key_syntax(key, MAX_NODE_KEY, "key")
                .map_err(|source| ViewError::Limit { index, source })?;
            if keys.insert(key) {
                Ok(())
            } else {
                Err(ViewError::DuplicateKey {
                    key: String::from(key),
                })
            }
        }
    }
}
