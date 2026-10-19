//! Actual tool slots. Every payload field is an independent aliasable bundle, so
//! a live patch shares unchanged payloads with the previous version.
mod claims;
mod patch;
use super::{
    bundle::{self, Body, BundleSlot},
    content::ContentSlot,
    slots::*,
};
use crate::model::{
    ids::{ProviderTerminalId, ToolId},
    json::{OwnedJson, lifetime::Shared},
    literal::LiteralEvidence,
    retention::{
        CensusPermit, RetentionError,
        document::{ConstructionRole, Publication},
        graph::Count,
    },
    tools::*,
};
pub use claims::{ClaimSlot, ClaimsSlot};

pub struct ToolSlot {
    id: BundleSlot<IdSlot<ToolId>>,
    title: BundleSlot<TextSlot>,
    name: Option<BundleSlot<TextSlot>>,
    kind: ToolKind,
    status: ToolStatus,
    output: BundleSlot<OutputsSlot>,
    locations: BundleSlot<LocationsSlot>,
    arguments: Option<BundleSlot<JsonSlot>>,
    result: Option<BundleSlot<JsonSlot>>,
    extensions: Option<BundleSlot<ExtensionSlot>>,
    claims: ClaimsSlot,
    literal: LiteralEvidence,
    display: Option<BundleSlot<DisplaySlot>>,
}
#[expect(
    clippy::large_enum_variant,
    reason = "tool bundles charge the inline output size; boxing would move bytes outside that charge"
)]
pub enum OutputSlot {
    Content {
        content: Box<ContentSlot>,
        extensions: Option<ExtensionSlot>,
    },
    Diff(DiffSlot),
    Terminal {
        id: TerminalSlot,
        extensions: Option<ExtensionSlot>,
    },
}
impl OutputSlot {
    pub fn wire(&self) -> impl serde::Serialize + '_ {
        super::wire::output_value(self)
    }
}

pub struct TerminalSlot {
    id: IdSlot<ProviderTerminalId>,
}
pub struct DiffSlot {
    path: PathSlot,
    previous: Option<TextSlot>,
    replacement: TextSlot,
    extensions: Option<ExtensionSlot>,
}
pub struct LocationSlot {
    path: PathSlot,
    line: Option<u32>,
    extensions: Option<ExtensionSlot>,
}
/// The output slab of one tool version; replaced only when output changes.
struct OutputsSlot(Vec<OutputSlot>);
/// The location slab of one tool version; replaced only when locations change.
struct LocationsSlot(Vec<LocationSlot>);
impl OutputsSlot {
    fn build(
        values: Vec<ToolOutput>,
        role: &ConstructionRole,
        original: &Shared<NativeCredit>,
        pool: &mut NativeSourcePool,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let mut builder = SlabBuilder::new(role)?;
        let values = builder.map_in(values, pool, |value, builder, pool| {
            OutputSlot::new(value, builder, original, pool)
        })?;
        Ok(BundleSlot::new(bundle::BundleOwner::build(
            Self(values),
            builder,
            original,
        )?))
    }
}
impl Body for OutputsSlot {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        count.add_exclusive(vector_bytes(&self.0)?)?;
        self.0.iter().try_for_each(|value| value.count(count))
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.0.iter().for_each(|value| value.cleanup(permit));
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        self.0.iter().for_each(|value| value.adopt(publication));
    }
    fn retire(&self) {
        self.0.iter().for_each(OutputSlot::retire);
    }
}
impl LocationsSlot {
    fn build(
        values: Vec<Location>,
        role: &ConstructionRole,
        original: &Shared<NativeCredit>,
        pool: &mut NativeSourcePool,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let mut builder = SlabBuilder::new(role)?;
        let values = builder.map_in(values, pool, |value, _, pool| {
            Ok(LocationSlot {
                path: PathSlot::new(value.path, pool)?,
                line: value.line,
                extensions: value.extensions.map(ExtensionSlot::new),
            })
        })?;
        Ok(BundleSlot::new(bundle::BundleOwner::build(
            Self(values),
            builder,
            original,
        )?))
    }
}
impl Body for LocationsSlot {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        count.add_exclusive(vector_bytes(&self.0)?)?;
        self.0.iter().try_for_each(|value| value.count(count))
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.0.iter().for_each(|value| value.cleanup(permit));
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        self.0.iter().for_each(|value| value.adopt(publication));
    }
    fn retire(&self) {
        self.0.iter().for_each(LocationSlot::retire);
    }
}
fn json_part(
    value: Option<OwnedJson>,
    role: &ConstructionRole,
    original: &Shared<NativeCredit>,
) -> Result<Option<BundleSlot<JsonSlot>>, PhysicalError> {
    value
        .map(|value| bundle::part(role, original, JsonSlot::new(value)))
        .transpose()
}
fn text_part(
    value: Option<String>,
    role: &ConstructionRole,
    original: &Shared<NativeCredit>,
    pool: &mut NativeSourcePool,
) -> Result<Option<BundleSlot<TextSlot>>, PhysicalError> {
    value
        .map(|value| bundle::part(role, original, TextSlot::new(value, pool)?))
        .transpose()
}
fn alias<T: Body>(value: &Option<BundleSlot<T>>) -> Option<BundleSlot<T>> {
    value.as_ref().map(BundleSlot::alias)
}
impl ToolSlot {
    pub(super) fn new(
        value: ToolRecord,
        role: &ConstructionRole,
        original: &Shared<NativeCredit>,
        pool: &mut NativeSourcePool,
    ) -> Result<Self, PhysicalError> {
        Ok(Self {
            id: bundle::part(role, original, IdSlot::tool(value.id))?,
            title: bundle::part(role, original, TextSlot::new(value.title, pool)?)?,
            name: text_part(value.name, role, original, pool)?,
            kind: value.kind,
            status: value.status,
            output: OutputsSlot::build(value.output, role, original, pool)?,
            locations: LocationsSlot::build(value.locations, role, original, pool)?,
            arguments: json_part(value.arguments, role, original)?,
            result: json_part(value.result, role, original)?,
            extensions: value
                .extensions
                .map(|value| bundle::part(role, original, ExtensionSlot::new(value)))
                .transpose()?,
            claims: ClaimsSlot::new(value.claims, role, original, pool)?,
            literal: value.literal,
            display: value
                .client_display
                .map(|value| bundle::part(role, original, DisplaySlot::new(value)))
                .transpose()?,
        })
    }
    /// A new version whose payload bundles alias this one; `status` is copied.
    fn alias(&self) -> Self {
        Self {
            id: self.id.alias(),
            title: self.title.alias(),
            name: alias(&self.name),
            kind: self.kind,
            status: self.status,
            output: self.output.alias(),
            locations: self.locations.alias(),
            arguments: alias(&self.arguments),
            result: alias(&self.result),
            extensions: alias(&self.extensions),
            claims: self.claims.alias(),
            literal: self.literal,
            display: alias(&self.display),
        }
    }
    pub fn id(&self) -> PayloadGuard<'_, ToolId> {
        self.id.body().read()
    }
    pub fn title(&self) -> TextGuard<'_> {
        self.title.body().read()
    }
    pub fn name(&self) -> Option<TextGuard<'_>> {
        self.name.as_ref().map(|value| value.body().read())
    }
    pub fn kind(&self) -> ToolKind {
        self.kind
    }
    pub fn status(&self) -> ToolStatus {
        self.status
    }
    pub(super) fn output_capacity(&self) -> usize {
        self.output.body().0.capacity()
    }
    pub(super) fn location_capacity(&self) -> usize {
        self.locations.body().0.capacity()
    }
    pub fn output(&self) -> &[OutputSlot] {
        &self.output.body().0
    }
    pub fn locations(&self) -> &[LocationSlot] {
        &self.locations.body().0
    }
    pub fn arguments(&self) -> Option<PayloadGuard<'_, OwnedJson>> {
        self.arguments.as_ref().map(|value| value.body().read())
    }
    pub fn result(&self) -> Option<PayloadGuard<'_, OwnedJson>> {
        self.result.as_ref().map(|value| value.body().read())
    }
    pub fn literal(&self) -> LiteralEvidence {
        self.literal
    }
    pub fn extensions(&self) -> Option<PayloadGuard<'_, crate::model::extensions::Extensions>> {
        self.extensions.as_ref().map(|value| value.body().read())
    }
    pub fn claims(&self) -> &ClaimsSlot {
        &self.claims
    }
    pub fn client_display(
        &self,
    ) -> Option<PayloadGuard<'_, crate::model::shared_display::SharedDisplay>> {
        self.display.as_ref().map(|value| value.body().read())
    }
    /// Visits every independent payload bundle of this version.
    fn each(&self, mut visit: impl FnMut(Part<'_>)) {
        visit(Part::Id(&self.id));
        visit(Part::Text(&self.title));
        if let Some(value) = &self.name {
            visit(Part::Text(value));
        }
        visit(Part::Outputs(&self.output));
        visit(Part::Locations(&self.locations));
        for value in [&self.arguments, &self.result].into_iter().flatten() {
            visit(Part::Json(value));
        }
        if let Some(value) = &self.extensions {
            visit(Part::Extensions(value));
        }
        if let Some(value) = &self.display {
            visit(Part::Display(value));
        }
    }
    pub(super) fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        let mut result = Ok(());
        self.each(|part| {
            if result.is_ok() {
                result = part.count(count);
            }
        });
        result?;
        self.claims.count(count)
    }
    pub(super) fn retire(&self) {
        self.each(|part| part.retire());
        self.claims.retire();
    }
    pub(super) fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.each(|part| part.cleanup(permit));
        self.claims.cleanup(permit);
    }
    pub(super) fn adopt(&self, publication: &mut Publication<'_>) {
        self.each(|part| part.adopt(publication));
        self.claims.adopt(publication);
    }
}
enum Part<'a> {
    Id(&'a BundleSlot<IdSlot<ToolId>>),
    Text(&'a BundleSlot<TextSlot>),
    Outputs(&'a BundleSlot<OutputsSlot>),
    Locations(&'a BundleSlot<LocationsSlot>),
    Json(&'a BundleSlot<JsonSlot>),
    Extensions(&'a BundleSlot<ExtensionSlot>),
    Display(&'a BundleSlot<DisplaySlot>),
}
macro_rules! each_part {
    ($part:expr, $value:ident => $body:expr) => {
        match $part {
            Part::Id($value) => $body,
            Part::Text($value) => $body,
            Part::Outputs($value) => $body,
            Part::Locations($value) => $body,
            Part::Json($value) => $body,
            Part::Extensions($value) => $body,
            Part::Display($value) => $body,
        }
    };
}
impl Part<'_> {
    fn count(self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        each_part!(self, value => value.count(count))
    }
    fn retire(self) {
        each_part!(self, value => value.retire())
    }
    fn cleanup(self, permit: &CensusPermit<'_>) {
        each_part!(self, value => value.cleanup(permit))
    }
    fn adopt(self, publication: &mut Publication<'_>) {
        each_part!(self, value => value.adopt(publication))
    }
}
impl OutputSlot {
    pub(super) fn matches(&self, value: &ToolOutput) -> bool {
        use super::content::same_extensions;
        match (self, value) {
            (
                Self::Content {
                    content,
                    extensions,
                },
                ToolOutput::Content {
                    content: right,
                    extensions: right_extensions,
                },
            ) => **content == **right && same_extensions(extensions, right_extensions),
            (Self::Diff(left), ToolOutput::FileDiff(right)) => {
                *left.path.read() == right.path
                    && left.previous.as_ref().map(TextSlot::read).as_deref()
                        == right.previous.as_deref()
                    && *left.replacement.read() == right.replacement
                    && same_extensions(&left.extensions, &right.extensions)
            }
            (
                Self::Terminal { id, extensions },
                ToolOutput::TerminalReference {
                    id: right,
                    extensions: right_extensions,
                },
            ) => *id.id() == *right && same_extensions(extensions, right_extensions),
            _ => false,
        }
    }
    fn new(
        value: ToolOutput,
        builder: &mut SlabBuilder<'_>,
        original: &Shared<NativeCredit>,
        pool: &mut NativeSourcePool,
    ) -> Result<Self, PhysicalError> {
        Ok(match value {
            ToolOutput::Content {
                content,
                extensions,
            } => {
                // One actual content Box, prepaid before allocation. The old
                // plain Box stays covered by its native receipt until adoption.
                builder.exclusive_box::<ContentSlot>()?;
                Self::Content {
                    content: Box::new(ContentSlot::new(
                        pool.unbox(content)?,
                        builder.role(),
                        original,
                        pool,
                    )?),
                    extensions: extensions.map(ExtensionSlot::new),
                }
            }
            ToolOutput::FileDiff(value) => Self::Diff(DiffSlot {
                path: PathSlot::new(value.path, pool)?,
                previous: optional_text(value.previous, pool)?,
                replacement: TextSlot::new(value.replacement, pool)?,
                extensions: value.extensions.map(ExtensionSlot::new),
            }),
            ToolOutput::TerminalReference { id, extensions } => Self::Terminal {
                id: TerminalSlot {
                    id: IdSlot::terminal(id),
                },
                extensions: extensions.map(ExtensionSlot::new),
            },
        })
    }
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        match self {
            Self::Content {
                content,
                extensions,
            } => {
                count.add_exclusive(std::mem::size_of::<ContentSlot>())?;
                content.count(count)?;
                if let Some(v) = extensions {
                    v.count(count)?;
                }
            }
            Self::Diff(v) => {
                v.path.count(count)?;
                if let Some(value) = &v.previous {
                    value.count(count)?;
                }
                v.replacement.count(count)?;
                if let Some(v) = &v.extensions {
                    v.count(count)?;
                }
            }
            Self::Terminal { id, extensions } => {
                id.id.count(count)?;
                if let Some(v) = extensions {
                    v.count(count)?;
                }
            }
        }
        Ok(())
    }
    fn retire(&self) {
        match self {
            Self::Content {
                content,
                extensions,
            } => {
                content.retire();
                if let Some(v) = extensions {
                    v.retire();
                }
            }
            Self::Diff(v) => {
                v.path.retire();
                retire_text(&v.previous);
                v.replacement.retire();
                if let Some(v) = &v.extensions {
                    v.retire();
                }
            }
            Self::Terminal { id, extensions } => {
                id.id.retire();
                if let Some(v) = extensions {
                    v.retire();
                }
            }
        }
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        match self {
            Self::Content {
                content,
                extensions,
            } => {
                content.cleanup(permit);
                if let Some(v) = extensions {
                    v.cleanup(permit);
                }
            }
            Self::Diff(v) => {
                v.path.cleanup(permit);
                v.replacement.cleanup(permit);
                if let Some(value) = &v.previous {
                    value.cleanup(permit);
                }
                if let Some(v) = &v.extensions {
                    v.cleanup(permit);
                }
            }
            Self::Terminal { id, extensions } => {
                id.id.cleanup(permit);
                if let Some(v) = extensions {
                    v.cleanup(permit);
                }
            }
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        match self {
            Self::Content {
                content,
                extensions,
            } => {
                content.adopt(publication);
                if let Some(v) = extensions {
                    v.adopt(publication);
                }
            }
            Self::Diff(v) => {
                v.path.adopt(publication);
                v.replacement.adopt(publication);
                if let Some(value) = &v.previous {
                    value.adopt(publication);
                }
                if let Some(v) = &v.extensions {
                    v.adopt(publication);
                }
            }
            Self::Terminal { id, extensions } => {
                id.id.adopt(publication);
                if let Some(v) = extensions {
                    v.adopt(publication);
                }
            }
        }
    }
}
impl LocationSlot {
    pub(super) fn matches(&self, value: &Location) -> bool {
        *self.path.read() == value.path
            && self.line == value.line
            && super::content::same_extensions(&self.extensions, &value.extensions)
    }
    pub fn path(&self) -> PathGuard<'_> {
        self.path.read()
    }
    pub fn line(&self) -> Option<u32> {
        self.line
    }
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.path.count(count)?;
        if let Some(v) = &self.extensions {
            v.count(count)?;
        }
        Ok(())
    }
    fn retire(&self) {
        self.path.retire();
        if let Some(v) = &self.extensions {
            v.retire();
        }
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.path.cleanup(permit);
        if let Some(v) = &self.extensions {
            v.cleanup(permit);
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        self.path.adopt(publication);
        if let Some(v) = &self.extensions {
            v.adopt(publication);
        }
    }
}

impl TerminalSlot {
    pub fn id(&self) -> PayloadGuard<'_, ProviderTerminalId> {
        self.id.read()
    }
}
impl DiffSlot {
    pub fn path(&self) -> PathGuard<'_> {
        self.path.read()
    }
    pub fn previous(&self) -> Option<TextGuard<'_>> {
        self.previous.as_ref().map(TextSlot::read)
    }
    pub fn replacement(&self) -> TextGuard<'_> {
        self.replacement.read()
    }
    pub fn extensions(&self) -> Option<PayloadGuard<'_, crate::model::extensions::Extensions>> {
        self.extensions.as_ref().map(ExtensionSlot::read)
    }
}
impl LocationSlot {
    pub fn extensions(&self) -> Option<PayloadGuard<'_, crate::model::extensions::Extensions>> {
        self.extensions.as_ref().map(ExtensionSlot::read)
    }
}
