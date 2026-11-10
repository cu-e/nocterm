//! Declarative views: the author's node list, checked once, drawn from snapshots.
//!
//! A package lists its nodes flat, each with the index of its parent, so a hostile file cannot
//! nest deeply enough to harm the parser. The list is compiled against the package's
//! declarations when it is read, and each draw expands it into a [`View`].

use alloc::{collections::BTreeSet, string::String, vec, vec::Vec};

use nocterm_plugin_model::SettingsSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{Context, MAX_PLACEHOLDERS, Output, Placeholder, Scope, Template, TemplateError};
use crate::{
    Glyph, LimitError, MAX_BADGE, MAX_BUTTON, MAX_DEPTH, MAX_LIST_ITEMS, MAX_NODE_KEY, MAX_NODES,
    MAX_TEXT, MIN_REFRESH_MS, Node, NodeKind, Refresh, Snapshot, Tone, View, ViewError,
    capability::Capability, capability::notifications::Own, error::key_syntax, sanitize_display,
};

/// The kinds of node an author writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NodeKindName {
    Row,
    Column,
    Text,
    Icon,
    Meter,
    Badge,
    Button,
    Toggle,
    List,
    Divider,
    Spacer,
    /// A titled group: `text` is the title, `detail` an optional count, `folded` how it starts.
    Section,
    /// A row of a list: `text` is the title, `detail` the muted line, `icon` leads it.
    Item,
    /// A small button with an `icon`; `text` names its action in a tooltip. Needs a key.
    IconButton,
    /// A line of small muted `text`.
    Caption,
    /// A `text` label and its `detail` value on one line.
    Field,
}

/// A node as written in a package.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct NodeDto {
    pub kind: Option<NodeKindName>,
    /// The index of the parent in the list, which must come earlier.
    #[serde(default)]
    pub parent: Option<u16>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub tone: Option<Tone>,
    /// An expression ending in `threshold(low, high)` that sets the tone.
    #[serde(default)]
    pub tone_when: Option<String>,
    /// A template: the text of a text, badge, button or caption, the title of a section or an
    /// item, the label of an icon button or a field.
    #[serde(default)]
    pub text: Option<String>,
    /// A template: the count of a section, the detail line of an item, the value of a field.
    #[serde(default)]
    pub detail: Option<String>,
    /// Whether a section starts folded; a section folds only when it has a key.
    #[serde(default)]
    pub folded: Option<bool>,
    #[serde(default)]
    pub icon: Option<Glyph>,
    /// An expression: a number for a meter, a switch value for a toggle.
    #[serde(default)]
    pub value: Option<String>,
    /// For a list: the observing capability whose items its children repeat for.
    #[serde(default)]
    pub each: Option<String>,
}

/// A view as written in a package.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ViewDto {
    #[serde(default)]
    pub refresh: Option<Refresh>,
    pub nodes: Vec<NodeDto>,
}

/// A view the package declares wrongly.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ViewTemplateError {
    #[error("a view has at most {MAX_NODES} nodes")]
    TooManyNodes,
    #[error("a view has at most {MAX_PLACEHOLDERS} placeholders")]
    TooManyPlaceholders,
    #[error("a refresh must wait at least {MIN_REFRESH_MS} ms")]
    Refresh,
    #[error("node {index} has no `kind`")]
    NoKind { index: usize },
    #[error("node {index}: `{field}` does not belong to a {kind} node")]
    Field {
        index: usize,
        field: &'static str,
        kind: &'static str,
    },
    #[error("node {index} needs `{field}`")]
    Missing { index: usize, field: &'static str },
    #[error("node {index} names a parent that does not come before it, or cannot hold children")]
    Parent { index: usize },
    #[error("node {index} is nested deeper than {MAX_DEPTH} levels")]
    TooDeep { index: usize },
    #[error("node {index}: `each` must name an observing capability that has items")]
    Each { index: usize },
    #[error("node {index}: buttons, toggles and keys cannot repeat inside a list with `each`")]
    RepeatedKey { index: usize },
    #[error("node {index}: its key is not valid")]
    Key {
        index: usize,
        #[source]
        source: LimitError,
    },
    #[error("node {index}: its key is already used by an earlier node")]
    DuplicateKey { index: usize },
    #[error("node {index}: {source}")]
    Template {
        index: usize,
        #[source]
        source: TemplateError,
    },
}

/// A view that could not be drawn this time.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum RenderError {
    #[error("the view needs more than {MAX_NODES} nodes")]
    TooManyNodes,
    #[error("the view is invalid")]
    Invalid(#[from] ViewError),
}

#[derive(Clone, Debug, PartialEq)]
enum Content {
    Row,
    Column,
    Text(Template),
    Icon(Glyph),
    Meter(Placeholder),
    Badge(Template),
    Button(Template),
    Toggle(Placeholder),
    List {
        each: bool,
    },
    Divider,
    Spacer,
    Section {
        title: Template,
        count: Option<Template>,
        folded: bool,
    },
    Item {
        title: Template,
        detail: Option<Template>,
        icon: Option<Glyph>,
    },
    IconButton {
        icon: Glyph,
        label: Template,
    },
    Caption(Template),
    Field {
        label: Template,
        value: Template,
    },
}

#[derive(Clone, Debug, PartialEq)]
struct Entry {
    parent: Option<u16>,
    key: Option<String>,
    tone: Tone,
    tone_when: Option<Placeholder>,
    content: Content,
}

/// A checked declarative view.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewTemplate {
    refresh: Option<Refresh>,
    entries: Vec<Entry>,
    children: Vec<Vec<usize>>,
}

impl ViewTemplate {
    /// Checks the nodes against what the package declares.
    pub fn compile(
        dto: &ViewDto,
        declared: &[&str],
        settings: &SettingsSchema,
    ) -> Result<Self, ViewTemplateError> {
        if dto.nodes.len() > MAX_NODES {
            return Err(ViewTemplateError::TooManyNodes);
        }
        if matches!(dto.refresh, Some(Refresh::AfterMs(ms)) if ms < MIN_REFRESH_MS) {
            return Err(ViewTemplateError::Refresh);
        }
        let mut entries: Vec<Entry> = Vec::with_capacity(dto.nodes.len());
        let mut depth = vec![1u8; dto.nodes.len()];
        let mut repeated = vec![false; dto.nodes.len()];
        let mut holes = 0;
        let mut keys = BTreeSet::new();
        for (index, node) in dto.nodes.iter().enumerate() {
            if let Some(parent) = node.parent {
                let parent = usize::from(parent);
                let holds = entries
                    .get(parent)
                    .is_some_and(|entry| entry.content.holds());
                if !holds {
                    return Err(ViewTemplateError::Parent { index });
                }
                depth[index] = depth[parent] + 1;
                repeated[index] = repeated[parent] || entries[parent].content.repeats();
                if depth[index] > MAX_DEPTH {
                    return Err(ViewTemplateError::TooDeep { index });
                }
            }
            let scope = Scope {
                declared,
                settings,
                item: repeated[index],
            };
            let entry = compile_node(index, node, &scope, repeated[index])?;
            if let Some(key) = &entry.key
                && !keys.insert(key.clone())
            {
                return Err(ViewTemplateError::DuplicateKey { index });
            }
            holes += entry.holes();
            if holes > MAX_PLACEHOLDERS {
                return Err(ViewTemplateError::TooManyPlaceholders);
            }
            entries.push(entry);
        }
        let mut children = vec![Vec::new(); entries.len()];
        for (index, entry) in entries.iter().enumerate() {
            if let Some(parent) = entry.parent {
                children[usize::from(parent)].push(index);
            }
        }
        Ok(Self {
            refresh: dto.refresh,
            entries,
            children,
        })
    }

    /// Draws the view for `snapshot`. The result is sanitized and valid, or an error.
    ///
    /// List expansion counts toward the node limit before a node is made.
    pub fn render(&self, snapshot: &Snapshot) -> Result<View, RenderError> {
        let mut nodes = Vec::new();
        let cx = Context {
            snapshot,
            item: None,
        };
        for (index, entry) in self.entries.iter().enumerate() {
            if entry.parent.is_none() {
                self.emit(index, None, &cx, &mut nodes)?;
            }
        }
        Ok(View {
            nodes,
            refresh: self.refresh,
        }
        .into_validated()?)
    }

    fn emit(
        &self,
        index: usize,
        parent: Option<u16>,
        cx: &Context<'_>,
        out: &mut Vec<Node>,
    ) -> Result<(), RenderError> {
        let entry = &self.entries[index];
        let at = u16::try_from(out.len()).map_err(|_| RenderError::TooManyNodes)?;
        if out.len() >= MAX_NODES {
            return Err(RenderError::TooManyNodes);
        }
        out.push(entry.node(parent, cx));
        if matches!(entry.content, Content::List { each: true }) {
            let items = match cx.snapshot.observation(Own::ID) {
                Some(Ok(crate::Observed::Notices(notices))) => notices.0.as_slice(),
                _ => &[],
            };
            for item in items.iter().take(MAX_LIST_ITEMS) {
                let inner = Context {
                    snapshot: cx.snapshot,
                    item: Some(item),
                };
                for child in &self.children[index] {
                    self.emit(*child, Some(at), &inner, out)?;
                }
            }
        } else {
            for child in &self.children[index] {
                self.emit(*child, Some(at), cx, out)?;
            }
        }
        Ok(())
    }
}

impl Content {
    fn holds(&self) -> bool {
        matches!(
            self,
            Self::Row | Self::Column | Self::List { .. } | Self::Section { .. } | Self::Item { .. }
        )
    }

    /// The templates the node renders, for the placeholder budget.
    fn templates(&self) -> [Option<&Template>; 2] {
        match self {
            Self::Text(template)
            | Self::Badge(template)
            | Self::Button(template)
            | Self::Caption(template) => [Some(template), None],
            Self::Section { title, count, .. } => [Some(title), count.as_ref()],
            Self::Item { title, detail, .. } => [Some(title), detail.as_ref()],
            Self::IconButton { label, .. } => [Some(label), None],
            Self::Field { label, value } => [Some(label), Some(value)],
            _ => [None, None],
        }
    }

    fn repeats(&self) -> bool {
        matches!(self, Self::List { each: true })
    }
}

impl Entry {
    fn holes(&self) -> usize {
        let own = match &self.content {
            Content::Meter(_) | Content::Toggle(_) => 1,
            content => content
                .templates()
                .into_iter()
                .flatten()
                .map(|template| template.placeholders().count())
                .sum(),
        };
        own + usize::from(self.tone_when.is_some())
    }

    fn node(&self, parent: Option<u16>, cx: &Context<'_>) -> Node {
        let kind = match &self.content {
            Content::Row => NodeKind::Row,
            Content::Column => NodeKind::Column,
            Content::Text(template) => {
                NodeKind::Text(sanitize_display(&template.render(cx), MAX_TEXT))
            }
            Content::Icon(icon) => NodeKind::Icon(*icon),
            Content::Meter(value) => {
                let percent = value.number(cx).unwrap_or(0.0).clamp(0.0, 100.0);
                NodeKind::Meter(percent as u8)
            }
            Content::Badge(template) => {
                NodeKind::Badge(sanitize_display(&template.render(cx), MAX_BADGE))
            }
            Content::Button(template) => {
                NodeKind::Button(sanitize_display(&template.render(cx), MAX_BUTTON))
            }
            Content::Toggle(value) => NodeKind::Toggle(value.switch(cx).unwrap_or(false)),
            Content::List { .. } => NodeKind::List,
            Content::Divider => NodeKind::Divider,
            Content::Spacer => NodeKind::Spacer,
            Content::Section {
                title,
                count,
                folded,
            } => NodeKind::Section {
                title: sanitize_display(&title.render(cx), MAX_BUTTON),
                count: count
                    .as_ref()
                    .map(|count| sanitize_display(&count.render(cx), MAX_BADGE)),
                folded: *folded,
            },
            Content::Item {
                title,
                detail,
                icon,
            } => NodeKind::Item {
                title: sanitize_display(&title.render(cx), MAX_TEXT),
                detail: detail
                    .as_ref()
                    .map(|detail| sanitize_display(&detail.render(cx), MAX_TEXT)),
                icon: *icon,
                selected: false,
            },
            Content::IconButton { icon, label } => NodeKind::IconButton {
                icon: *icon,
                label: sanitize_display(&label.render(cx), MAX_BUTTON),
            },
            Content::Caption(text) => {
                NodeKind::Caption(sanitize_display(&text.render(cx), MAX_TEXT))
            }
            Content::Field { label, value } => NodeKind::Field {
                label: sanitize_display(&label.render(cx), MAX_BUTTON),
                value: sanitize_display(&value.render(cx), MAX_TEXT),
            },
        };
        let tone = match &self.tone_when {
            Some(expression) => expression.tone(cx).unwrap_or(Tone::Muted),
            None => self.tone,
        };
        Node {
            parent,
            key: self.key.clone(),
            kind,
            tone,
        }
    }
}

fn compile_node(
    index: usize,
    node: &NodeDto,
    scope: &Scope<'_>,
    repeated: bool,
) -> Result<Entry, ViewTemplateError> {
    let kind = node.kind.ok_or(ViewTemplateError::NoKind { index })?;
    check_fields(index, node, kind)?;
    if repeated
        && (node.key.is_some()
            || matches!(
                kind,
                NodeKindName::Button | NodeKindName::Toggle | NodeKindName::IconButton
            ))
    {
        return Err(ViewTemplateError::RepeatedKey { index });
    }
    let reader = Reader { index, node, scope };
    let content = reader.content(kind)?;
    let tone_when = reader.tone_when()?;
    if let Some(key) = &node.key {
        key_syntax(key, MAX_NODE_KEY, "key")
            .map_err(|source| ViewTemplateError::Key { index, source })?;
    }
    if matches!(
        content,
        Content::Button(_) | Content::Toggle(_) | Content::IconButton { .. }
    ) && node.key.is_none()
    {
        return Err(ViewTemplateError::Missing {
            index,
            field: "key",
        });
    }
    Ok(Entry {
        parent: node.parent,
        key: node.key.clone(),
        tone: node.tone.unwrap_or_default(),
        tone_when,
        content,
    })
}

/// Reads the fields of one written node against the package's declarations.
struct Reader<'a> {
    index: usize,
    node: &'a NodeDto,
    scope: &'a Scope<'a>,
}

impl Reader<'_> {
    fn missing(&self, field: &'static str) -> ViewTemplateError {
        ViewTemplateError::Missing {
            index: self.index,
            field,
        }
    }

    fn parse(&self, text: &str) -> Result<Template, ViewTemplateError> {
        let index = self.index;
        let template = Template::parse(text)
            .map_err(|source| ViewTemplateError::Template { index, source })?;
        template
            .check(self.scope)
            .map_err(|source| ViewTemplateError::Template { index, source })?;
        Ok(template)
    }

    /// The `text` template, which the kind needs.
    fn text(&self) -> Result<Template, ViewTemplateError> {
        let text = self.node.text.as_deref().ok_or(self.missing("text"))?;
        self.parse(text)
    }

    /// The `detail` template, if the node has one.
    fn detail(&self) -> Result<Option<Template>, ViewTemplateError> {
        self.node
            .detail
            .as_deref()
            .map(|text| self.parse(text))
            .transpose()
    }

    fn icon(&self) -> Result<crate::Glyph, ViewTemplateError> {
        self.node.icon.ok_or(self.missing("icon"))
    }

    fn hole(&self, text: &str, wanted: Output) -> Result<Placeholder, ViewTemplateError> {
        let index = self.index;
        let hole = Placeholder::parse(text)
            .map_err(|source| ViewTemplateError::Template { index, source })?;
        hole.expect(self.scope, wanted)
            .map_err(|source| ViewTemplateError::Template { index, source })?;
        Ok(hole)
    }

    /// The `value` expression, which a meter or a toggle needs.
    fn value(&self, wanted: Output) -> Result<Placeholder, ViewTemplateError> {
        let text = self.node.value.as_deref().ok_or(self.missing("value"))?;
        self.hole(text, wanted)
    }

    fn tone_when(&self) -> Result<Option<Placeholder>, ViewTemplateError> {
        self.node
            .tone_when
            .as_deref()
            .map(|text| self.hole(text, Output::Tone))
            .transpose()
    }

    fn content(&self, kind: NodeKindName) -> Result<Content, ViewTemplateError> {
        Ok(match kind {
            NodeKindName::Row => Content::Row,
            NodeKindName::Column => Content::Column,
            NodeKindName::Text => Content::Text(self.text()?),
            NodeKindName::Badge => Content::Badge(self.text()?),
            NodeKindName::Button => Content::Button(self.text()?),
            NodeKindName::Icon => Content::Icon(self.icon()?),
            NodeKindName::Meter => Content::Meter(self.value(Output::Number)?),
            NodeKindName::Toggle => Content::Toggle(self.value(Output::Bool)?),
            NodeKindName::List => Content::List {
                each: each(self.index, self.node, self.scope)?,
            },
            NodeKindName::Divider => Content::Divider,
            NodeKindName::Spacer => Content::Spacer,
            NodeKindName::Section => Content::Section {
                title: self.text()?,
                count: self.detail()?,
                folded: self.node.folded.unwrap_or(false),
            },
            NodeKindName::Item => Content::Item {
                title: self.text()?,
                detail: self.detail()?,
                icon: self.node.icon,
            },
            NodeKindName::IconButton => Content::IconButton {
                icon: self.icon()?,
                label: self.text()?,
            },
            NodeKindName::Caption => Content::Caption(self.text()?),
            NodeKindName::Field => Content::Field {
                label: self.text()?,
                value: self.detail()?.ok_or(self.missing("detail"))?,
            },
        })
    }
}

fn each(index: usize, node: &NodeDto, scope: &Scope<'_>) -> Result<bool, ViewTemplateError> {
    match node.each.as_deref() {
        None => Ok(false),
        Some(id) if id == Own::ID && scope.declared.contains(&id) => Ok(true),
        Some(_) => Err(ViewTemplateError::Each { index }),
    }
}

fn check_fields(index: usize, node: &NodeDto, kind: NodeKindName) -> Result<(), ViewTemplateError> {
    let name = match kind {
        NodeKindName::Row => "row",
        NodeKindName::Column => "column",
        NodeKindName::Text => "text",
        NodeKindName::Icon => "icon",
        NodeKindName::Meter => "meter",
        NodeKindName::Badge => "badge",
        NodeKindName::Button => "button",
        NodeKindName::Toggle => "toggle",
        NodeKindName::List => "list",
        NodeKindName::Divider => "divider",
        NodeKindName::Spacer => "spacer",
        NodeKindName::Section => "section",
        NodeKindName::Item => "item",
        NodeKindName::IconButton => "icon-button",
        NodeKindName::Caption => "caption",
        NodeKindName::Field => "field",
    };
    let text = matches!(
        kind,
        NodeKindName::Text
            | NodeKindName::Badge
            | NodeKindName::Button
            | NodeKindName::Section
            | NodeKindName::Item
            | NodeKindName::IconButton
            | NodeKindName::Caption
            | NodeKindName::Field
    );
    let detail = matches!(
        kind,
        NodeKindName::Section | NodeKindName::Item | NodeKindName::Field
    );
    let icon = matches!(
        kind,
        NodeKindName::Icon | NodeKindName::Item | NodeKindName::IconButton
    );
    let value = matches!(kind, NodeKindName::Meter | NodeKindName::Toggle);
    let stray = [
        ("text", node.text.is_some() && !text),
        ("detail", node.detail.is_some() && !detail),
        (
            "folded",
            node.folded.is_some() && kind != NodeKindName::Section,
        ),
        ("icon", node.icon.is_some() && !icon),
        ("value", node.value.is_some() && !value),
        ("each", node.each.is_some() && kind != NodeKindName::List),
    ];
    match stray.into_iter().find(|(_, stray)| *stray) {
        Some((field, _)) => Err(ViewTemplateError::Field {
            index,
            field,
            kind: name,
        }),
        None => Ok(()),
    }
}
