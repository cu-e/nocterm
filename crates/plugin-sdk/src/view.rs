//! Builds the arena a view is. The builder does not validate: the host does, and
//! `testing::Host` reports what it would refuse.

use alloc::{string::String, vec::Vec};

use nocterm_plugin_api::{Node, NodeKind, PluginIcon, Refresh, Tone, View};

/// The place of a node in the view being built, which is the parent a later node names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeId(u16);

/// A view under construction. Every method that adds a node takes its parent (`None` for a
/// root) and returns the new node's handle.
///
/// ```
/// use nocterm_plugin_sdk::ViewBuilder;
///
/// let mut view = ViewBuilder::new();
/// let row = view.row(None);
/// view.text(row, "12 min");
/// view.button(row, "reset", "Reset");
/// assert_eq!(view.build().nodes.len(), 3);
/// ```
#[derive(Clone, Debug, Default)]
pub struct ViewBuilder {
    nodes: Vec<Node>,
    refresh: Option<Refresh>,
}

impl ViewBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    fn add(&mut self, parent: impl Into<Option<NodeId>>, kind: NodeKind) -> NodeId {
        let id = NodeId(u16::try_from(self.nodes.len()).unwrap_or(u16::MAX));
        let mut node = Node::new(kind);
        node.parent = parent.into().map(|parent| parent.0);
        self.nodes.push(node);
        id
    }

    fn keyed(&mut self, parent: impl Into<Option<NodeId>>, kind: NodeKind, key: &str) -> NodeId {
        let id = self.add(parent, kind);
        self.key(id, key);
        id
    }

    /// A horizontal run of children.
    pub fn row(&mut self, parent: impl Into<Option<NodeId>>) -> NodeId {
        self.add(parent, NodeKind::Row)
    }

    /// A vertical run of children.
    pub fn column(&mut self, parent: impl Into<Option<NodeId>>) -> NodeId {
        self.add(parent, NodeKind::Column)
    }

    /// A host-scrolled run of children.
    pub fn list(&mut self, parent: impl Into<Option<NodeId>>) -> NodeId {
        self.add(parent, NodeKind::List)
    }

    pub fn text(&mut self, parent: impl Into<Option<NodeId>>, text: impl Into<String>) -> NodeId {
        self.add(parent, NodeKind::Text(text.into()))
    }

    pub fn icon(&mut self, parent: impl Into<Option<NodeId>>, icon: PluginIcon) -> NodeId {
        self.add(parent, NodeKind::Icon(icon))
    }

    /// A bar from 0 to 100.
    pub fn meter(&mut self, parent: impl Into<Option<NodeId>>, percent: u8) -> NodeId {
        self.add(parent, NodeKind::Meter(percent))
    }

    pub fn badge(&mut self, parent: impl Into<Option<NodeId>>, text: impl Into<String>) -> NodeId {
        self.add(parent, NodeKind::Badge(text.into()))
    }

    /// A button. Its `key` comes back in the `Activated` event.
    pub fn button(
        &mut self,
        parent: impl Into<Option<NodeId>>,
        key: &str,
        label: impl Into<String>,
    ) -> NodeId {
        self.keyed(parent, NodeKind::Button(label.into()), key)
    }

    /// A switch. Its `key` comes back in the `Toggled` event.
    pub fn toggle(&mut self, parent: impl Into<Option<NodeId>>, key: &str, on: bool) -> NodeId {
        self.keyed(parent, NodeKind::Toggle(on), key)
    }

    pub fn divider(&mut self, parent: impl Into<Option<NodeId>>) -> NodeId {
        self.add(parent, NodeKind::Divider)
    }

    pub fn spacer(&mut self, parent: impl Into<Option<NodeId>>) -> NodeId {
        self.add(parent, NodeKind::Spacer)
    }

    /// Colours a node. The application takes every tone from its theme.
    pub fn tone(&mut self, node: NodeId, tone: Tone) -> &mut Self {
        if let Some(node) = self.nodes.get_mut(usize::from(node.0)) {
            node.tone = tone;
        }
        self
    }

    /// Gives a node a key, which every key of the view must differ from.
    pub fn key(&mut self, node: NodeId, key: &str) -> &mut Self {
        if let Some(node) = self.nodes.get_mut(usize::from(node.0)) {
            node.key = Some(String::from(key));
        }
        self
    }

    /// Asks the host to draw the view again after a delay.
    pub fn refresh(&mut self, refresh: Refresh) -> &mut Self {
        self.refresh = Some(refresh);
        self
    }

    pub fn build(self) -> View {
        View {
            nodes: self.nodes,
            refresh: self.refresh,
        }
    }
}
