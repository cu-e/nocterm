//! What a plugin draws: a bounded arena of nodes.
//!
//! A view is plain data. The host sanitizes every string, validates the arena, and draws it
//! inside a frame it controls. There is no input, image, link, URL or drag node.

use alloc::{collections::BTreeSet, string::String, vec, vec::Vec};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{LimitError, PluginIcon, error::key_syntax, sanitize_display};

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
/// Shortest refresh delay a view may ask for.
pub const MIN_REFRESH_MS: u32 = 250;

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
    Icon(PluginIcon),
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
        }
    }

    /// The names of all kinds, for the API surface.
    pub const NAMES: [&'static str; 11] = [
        "row", "column", "text", "icon", "meter", "badge", "button", "toggle", "list", "divider",
        "spacer",
    ];

    fn holds_children(&self) -> bool {
        matches!(self, Self::Row | Self::Column | Self::List)
    }

    fn needs_key(&self) -> bool {
        matches!(self, Self::Button(_) | Self::Toggle(_))
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
        NodeKind::Meter(value) if *value > 100 => Err(limit(LimitError::Range {
            field: "meter",
            max: 100,
        })),
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
