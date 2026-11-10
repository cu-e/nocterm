//! Builds the arena a view is. The builder does not validate: the host does, and
//! `testing::Host` reports what it would refuse.
//!
//! Panels read best when they are built from the components the application's own panels use:
//! [`Section`]s of [`Item`] rows with [`ViewBuilder::icon_button`]s, [`ViewBuilder::caption`]s
//! for hints, [`ViewBuilder::field`]s for facts and [`Chart`]s of recent values. The host draws
//! them as it draws its own.

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use core::fmt::Display;

use nocterm_plugin_api::{Glyph, Node, NodeKind, Refresh, Tone, View};

/// The place of a node in the view being built, which is the parent a later node names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeId(u16);

/// A titled group of a panel, headed as the application heads its own sections.
///
/// ```
/// use nocterm_plugin_sdk::{Section, ViewBuilder};
///
/// let mut view = ViewBuilder::new();
/// let servers = view.section(None, Section::new("Servers").count(3).foldable("servers", false));
/// view.caption(servers, "No server is running.");
/// assert_eq!(view.build().nodes.len(), 2);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    title: String,
    count: Option<String>,
    fold: Option<(String, bool)>,
}

impl Section {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            count: None,
            fold: None,
        }
    }

    /// A count at the end of the heading, such as how many items the section holds.
    pub fn count(mut self, count: impl Display) -> Self {
        self.count = Some(count.to_string());
        self
    }

    /// Lets the user fold the section. The host keeps whether it is folded; `folded` is how it
    /// starts, and `key` must differ from every other key of the view.
    pub fn foldable(mut self, key: &str, folded: bool) -> Self {
        self.fold = Some((String::from(key), folded));
        self
    }
}

/// Recent values as a small area chart, drawn as the application draws its own graphs: a line
/// over a light fill per series, the newest value at the right end.
///
/// ```
/// use nocterm_plugin_sdk::{Chart, Section, ViewBuilder};
///
/// let mut view = ViewBuilder::new();
/// let cpu = view.section(None, Section::new("CPU").count("36%"));
/// view.chart(cpu, Chart::new([12, 30, 36]).max(100).span(60));
/// let network = view.section(None, Section::new("Network"));
/// view.chart(network, Chart::new([4, 9, 7]).and([2, 3, 5]));
/// assert_eq!(view.build().nodes.len(), 4);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chart {
    series: Vec<Vec<u32>>,
    max: Option<u32>,
    span: Option<u16>,
}

impl Chart {
    /// A chart of one series, oldest value first. It takes the node's tone.
    pub fn new(values: impl IntoIterator<Item = u32>) -> Self {
        Self {
            series: alloc::vec![values.into_iter().collect()],
            max: None,
            span: None,
        }
    }

    /// A second series drawn over the first, in the application's second chart colour, such as
    /// what was sent beside what was received.
    pub fn and(mut self, values: impl IntoIterator<Item = u32>) -> Self {
        self.series.push(values.into_iter().collect());
        self
    }

    /// The value at the top of the chart, such as 100 for a percentage. Without it the largest
    /// value is.
    pub fn max(mut self, max: u32) -> Self {
        self.max = Some(max);
        self
    }

    /// How many values the chart's width holds, at most 120. Fewer grow in from the right;
    /// without it the values given fill the width.
    pub fn span(mut self, span: u16) -> Self {
        self.span = Some(span);
        self
    }
}

/// A row of a list, drawn as the application draws its own: an icon, a title and a muted detail
/// line. Children added under it (icon buttons, badges) follow at its end.
///
/// ```
/// use nocterm_plugin_sdk::{Item, ViewBuilder, api::{Glyph, Tone}};
///
/// let mut view = ViewBuilder::new();
/// let list = view.list(None);
/// let row = view.item(
///     list,
///     Item::new("Survival").detail("on · de-1").icon(Glyph::Server).tone(Tone::Success).key("open.1"),
/// );
/// view.icon_button(row, "stop.1", Glyph::Stop, "Stop");
/// assert_eq!(view.build().nodes.len(), 3);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    title: String,
    detail: Option<String>,
    icon: Option<Glyph>,
    selected: bool,
    key: Option<String>,
    tone: Tone,
}

impl Item {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            detail: None,
            icon: None,
            selected: false,
            key: None,
            tone: Tone::Default,
        }
    }

    /// The muted line under the title.
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// The icon that leads the row, drawn in the row's tone.
    pub fn icon(mut self, icon: Glyph) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Marks the current row of a list.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Makes the whole row a control: a click reports `Activated` with `key`.
    pub fn key(mut self, key: &str) -> Self {
        self.key = Some(String::from(key));
        self
    }

    /// The colour of the icon, such as the state of what the row shows.
    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }
}

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

    pub fn icon(&mut self, parent: impl Into<Option<NodeId>>, icon: Glyph) -> NodeId {
        self.add(parent, NodeKind::Icon(icon))
    }

    /// A bar from 0 to 100.
    pub fn meter(&mut self, parent: impl Into<Option<NodeId>>, percent: u8) -> NodeId {
        self.add(parent, NodeKind::Meter(percent))
    }

    /// A small area chart of recent values.
    pub fn chart(&mut self, parent: impl Into<Option<NodeId>>, chart: Chart) -> NodeId {
        self.add(
            parent,
            NodeKind::Chart {
                series: chart.series,
                max: chart.max,
                span: chart.span,
            },
        )
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

    /// Lines of text in a monospace block the host scrolls, such as a file's contents.
    pub fn code(&mut self, parent: impl Into<Option<NodeId>>, text: impl Into<String>) -> NodeId {
        self.add(parent, NodeKind::Code(text.into()))
    }

    /// A titled group; nodes added under it are its content.
    pub fn section(&mut self, parent: impl Into<Option<NodeId>>, section: Section) -> NodeId {
        let (key, folded) = match section.fold {
            Some((key, folded)) => (Some(key), folded),
            None => (None, false),
        };
        let id = self.add(
            parent,
            NodeKind::Section {
                title: section.title,
                count: section.count,
                folded,
            },
        );
        if let Some(key) = key {
            self.key(id, &key);
        }
        id
    }

    /// A row of a list; nodes added under it follow at its end.
    pub fn item(&mut self, parent: impl Into<Option<NodeId>>, item: Item) -> NodeId {
        let id = self.add(
            parent,
            NodeKind::Item {
                title: item.title,
                detail: item.detail,
                icon: item.icon,
                selected: item.selected,
            },
        );
        self.tone(id, item.tone);
        if let Some(key) = item.key {
            self.key(id, &key);
        }
        id
    }

    /// A small borderless button with an icon; `label` names its action in a tooltip. Its
    /// `key` comes back in the `Activated` event.
    pub fn icon_button(
        &mut self,
        parent: impl Into<Option<NodeId>>,
        key: &str,
        icon: Glyph,
        label: impl Into<String>,
    ) -> NodeId {
        self.keyed(
            parent,
            NodeKind::IconButton {
                icon,
                label: label.into(),
            },
            key,
        )
    }

    /// A line of small muted text: a hint, an empty list, a state.
    pub fn caption(
        &mut self,
        parent: impl Into<Option<NodeId>>,
        text: impl Into<String>,
    ) -> NodeId {
        self.add(parent, NodeKind::Caption(text.into()))
    }

    /// A label and its value on one line. The node's tone colours the value.
    pub fn field(
        &mut self,
        parent: impl Into<Option<NodeId>>,
        label: impl Into<String>,
        value: impl Display,
    ) -> NodeId {
        self.add(
            parent,
            NodeKind::Field {
                label: label.into(),
                value: format!("{value}"),
            },
        )
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
