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
    LimitError, MAX_BADGE, MAX_BUTTON, MAX_DEPTH, MAX_LIST_ITEMS, MAX_NODE_KEY, MAX_NODES,
    MAX_TEXT, MIN_REFRESH_MS, Node, NodeKind, PluginIcon, Refresh, Snapshot, Tone, View, ViewError,
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
    /// A template, for text, badge and button.
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub icon: Option<PluginIcon>,
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
    Icon(PluginIcon),
    Meter(Placeholder),
    Badge(Template),
    Button(Template),
    Toggle(Placeholder),
    List { each: bool },
    Divider,
    Spacer,
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
        matches!(self, Self::Row | Self::Column | Self::List { .. })
    }

    fn repeats(&self) -> bool {
        matches!(self, Self::List { each: true })
    }
}

impl Entry {
    fn holes(&self) -> usize {
        let own = match &self.content {
            Content::Text(template) | Content::Badge(template) | Content::Button(template) => {
                template.placeholders().count()
            }
            Content::Meter(_) | Content::Toggle(_) => 1,
            _ => 0,
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
    let template = |text: &Option<String>| -> Result<Template, ViewTemplateError> {
        let text = text.as_deref().ok_or(ViewTemplateError::Missing {
            index,
            field: "text",
        })?;
        let template = Template::parse(text)
            .map_err(|source| ViewTemplateError::Template { index, source })?;
        template
            .check(scope)
            .map_err(|source| ViewTemplateError::Template { index, source })?;
        Ok(template)
    };
    let expression = |text: &Option<String>, wanted| -> Result<Placeholder, ViewTemplateError> {
        let text = text.as_deref().ok_or(ViewTemplateError::Missing {
            index,
            field: "value",
        })?;
        let hole = Placeholder::parse(text)
            .map_err(|source| ViewTemplateError::Template { index, source })?;
        hole.expect(scope, wanted)
            .map_err(|source| ViewTemplateError::Template { index, source })?;
        Ok(hole)
    };
    check_fields(index, node, kind)?;
    if repeated
        && (node.key.is_some() || matches!(kind, NodeKindName::Button | NodeKindName::Toggle))
    {
        return Err(ViewTemplateError::RepeatedKey { index });
    }
    let content = match kind {
        NodeKindName::Row => Content::Row,
        NodeKindName::Column => Content::Column,
        NodeKindName::Text => Content::Text(template(&node.text)?),
        NodeKindName::Badge => Content::Badge(template(&node.text)?),
        NodeKindName::Button => Content::Button(template(&node.text)?),
        NodeKindName::Icon => Content::Icon(node.icon.ok_or(ViewTemplateError::Missing {
            index,
            field: "icon",
        })?),
        NodeKindName::Meter => Content::Meter(expression(&node.value, Output::Number)?),
        NodeKindName::Toggle => Content::Toggle(expression(&node.value, Output::Bool)?),
        NodeKindName::List => Content::List {
            each: each(index, node, scope)?,
        },
        NodeKindName::Divider => Content::Divider,
        NodeKindName::Spacer => Content::Spacer,
    };
    let tone_when = node
        .tone_when
        .as_deref()
        .map(|text| {
            let hole = Placeholder::parse(text)
                .map_err(|source| ViewTemplateError::Template { index, source })?;
            hole.expect(scope, Output::Tone)
                .map_err(|source| ViewTemplateError::Template { index, source })?;
            Ok(hole)
        })
        .transpose()?;
    if let Some(key) = &node.key {
        key_syntax(key, MAX_NODE_KEY, "key")
            .map_err(|source| ViewTemplateError::Key { index, source })?;
    }
    if matches!(content, Content::Button(_) | Content::Toggle(_)) && node.key.is_none() {
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
    };
    let text = matches!(
        kind,
        NodeKindName::Text | NodeKindName::Badge | NodeKindName::Button
    );
    let value = matches!(kind, NodeKindName::Meter | NodeKindName::Toggle);
    let stray = [
        ("text", node.text.is_some() && !text),
        ("icon", node.icon.is_some() && kind != NodeKindName::Icon),
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
