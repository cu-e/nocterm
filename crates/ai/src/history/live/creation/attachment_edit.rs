//! Closed attachment edits copy only borrowed original facts before atomic graph publication.
use crate::{
    history::{
        SavedAttachment,
        live::inputs::{LiveAttachment, LocalItemKey},
        physical::{AttachmentSlot, LiveAttachmentSlot, LiveInputs, PhysicalError},
    },
    model::{
        json::deferred::{DecodeLedger, JsonDecodeError},
        native::{NativeList, NativeOwned},
        retention::document::ConstructionRole,
    },
};

#[derive(Clone, Copy)]
pub enum LiveAttachmentRef<'a> {
    Connection(&'a str),
    Group(&'a str),
    UnavailableLocal(&'a str),
    LocalItem(LocalItemKey),
}
/// The feature resolves these only against existing original workspace items.
pub struct LocalAttachmentFact<'a> {
    pub key: LocalItemKey,
    pub saved: super::super::inputs::SavedAttachmentRef<'a>,
}
pub enum AttachmentEdit<'a> {
    Toggle(LiveAttachmentRef<'a>),
    AddIfAbsent(LiveAttachmentRef<'a>),
    RemoveIndex(usize),
    Clear,
}
pub(in crate::history::live) struct PreparedAttachments {
    live: NativeOwned<Vec<LiveAttachment>>,
    saved: Option<NativeOwned<Vec<SavedAttachment>>>,
    removed: bool,
}
impl PreparedAttachments {
    pub(in crate::history::live) fn into_parts(
        self,
    ) -> (
        NativeOwned<Vec<LiveAttachment>>,
        Option<NativeOwned<Vec<SavedAttachment>>>,
        bool,
    ) {
        (self.live, self.saved, self.removed)
    }
}
pub(in crate::history::live) struct AttachmentPreparation<'a> {
    current: CurrentAttachments<'a>,
    remove: Option<usize>,
    clear: bool,
    add: Option<LiveAttachmentRef<'a>>,
    changed: bool,
    editing: bool,
    locals: &'a [LocalAttachmentFact<'a>],
}
impl<'a> AttachmentPreparation<'a> {
    pub(in crate::history::live) fn new(
        live: Option<&'a LiveInputs>,
        saved: &'a [AttachmentSlot],
        edit: AttachmentEdit<'a>,
        locals: &'a [LocalAttachmentFact<'a>],
    ) -> Self {
        let current = CurrentAttachments { live, saved };
        let (remove, add, clear, changed) = match edit {
            AttachmentEdit::Toggle(value) => match current.position(value) {
                Some(index) => (Some(index), None, false, true),
                None => (None, Some(value), false, true),
            },
            AttachmentEdit::AddIfAbsent(value) => {
                let changed = current.position(value).is_none();
                (None, changed.then_some(value), false, changed)
            }
            AttachmentEdit::RemoveIndex(index) => {
                let changed = index < current.len();
                (changed.then_some(index), None, false, changed)
            }
            AttachmentEdit::Clear => (None, None, true, current.len() != 0),
        };
        Self {
            current,
            remove,
            clear,
            add,
            changed,
            editing: live.is_some_and(LiveInputs::editing),
            locals,
        }
    }
    pub(in crate::history::live) fn changed(&self) -> bool {
        self.changed
    }
    pub(in crate::history::live) fn prepare(
        self,
        role: &ConstructionRole,
    ) -> Result<PreparedAttachments, PhysicalError> {
        let length = if self.clear {
            0
        } else {
            self.current.len() - usize::from(self.remove.is_some())
                + usize::from(self.add.is_some())
        };
        if length > 1024 {
            return Err(PhysicalError::QueueLimit);
        }
        let ledger = DecodeLedger::for_operation(role, crate::history::MAX_RESIDENT_BYTES)?;
        let mut live = NativeList::new(&ledger, length)?;
        let mut saved = (!self.editing)
            .then(|| NativeList::new(&ledger, length))
            .transpose()?;
        if !self.clear {
            for (index, current) in self.current.iter().enumerate() {
                if self.remove == Some(index) {
                    continue;
                }
                match current {
                    CurrentAttachment::Live(value) => {
                        self.push_live(value, &ledger, &mut live, saved.as_mut())?
                    }
                    CurrentAttachment::Saved(value) => {
                        self.push_saved(value, &ledger, &mut live, saved.as_mut())?
                    }
                }
            }
            if let Some(value) = self.add {
                self.push(value, &ledger, &mut live, saved.as_mut())?;
            }
        }
        Ok(PreparedAttachments {
            live: live.finish(),
            saved: saved.map(NativeList::finish),
            removed: self.clear || self.remove.is_some(),
        })
    }
    fn push_live(
        &self,
        value: &LiveAttachmentSlot,
        ledger: &DecodeLedger,
        live: &mut NativeList<LiveAttachment>,
        saved: Option<&mut NativeList<SavedAttachment>>,
    ) -> Result<(), JsonDecodeError> {
        match value {
            LiveAttachmentSlot::Connection(text) => self.push(
                LiveAttachmentRef::Connection(&text.read()),
                ledger,
                live,
                saved,
            ),
            LiveAttachmentSlot::Group(text) => {
                self.push(LiveAttachmentRef::Group(&text.read()), ledger, live, saved)
            }
            LiveAttachmentSlot::UnavailableLocal(text) => self.push(
                LiveAttachmentRef::UnavailableLocal(&text.read()),
                ledger,
                live,
                saved,
            ),
            LiveAttachmentSlot::LocalItem(key) => {
                self.push(LiveAttachmentRef::LocalItem(*key), ledger, live, saved)
            }
        }
    }
    fn push_saved(
        &self,
        value: &AttachmentSlot,
        ledger: &DecodeLedger,
        live: &mut NativeList<LiveAttachment>,
        saved: Option<&mut NativeList<SavedAttachment>>,
    ) -> Result<(), JsonDecodeError> {
        match value {
            AttachmentSlot::Connection(text) => self.push(
                LiveAttachmentRef::Connection(&text.read()),
                ledger,
                live,
                saved,
            ),
            AttachmentSlot::Group(text) => {
                self.push(LiveAttachmentRef::Group(&text.read()), ledger, live, saved)
            }
            AttachmentSlot::LocalTerminal(text) => self.push(
                LiveAttachmentRef::UnavailableLocal(&text.read()),
                ledger,
                live,
                saved,
            ),
        }
    }
    fn push(
        &self,
        value: LiveAttachmentRef<'_>,
        ledger: &DecodeLedger,
        live: &mut NativeList<LiveAttachment>,
        saved: Option<&mut NativeList<SavedAttachment>>,
    ) -> Result<(), JsonDecodeError> {
        let native = match value {
            LiveAttachmentRef::Connection(text) => {
                crate::history::live::inputs::connection(super::copied(ledger, text)?)
            }
            LiveAttachmentRef::Group(text) => {
                crate::history::live::inputs::group(super::copied(ledger, text)?)
            }
            LiveAttachmentRef::UnavailableLocal(text) => {
                crate::history::live::inputs::unavailable_local(super::copied(ledger, text)?)
            }
            LiveAttachmentRef::LocalItem(key) => {
                crate::history::live::inputs::local_item(key, ledger)?
            }
        };
        live.push(native)?;
        if let Some(saved) = saved {
            let fact = match value {
                LiveAttachmentRef::Connection(text) => {
                    Some(super::super::inputs::SavedAttachmentRef::Connection(text))
                }
                LiveAttachmentRef::Group(text) => {
                    Some(super::super::inputs::SavedAttachmentRef::Group(text))
                }
                LiveAttachmentRef::UnavailableLocal(text) => Some(
                    super::super::inputs::SavedAttachmentRef::LocalTerminal(text),
                ),
                LiveAttachmentRef::LocalItem(key) => self
                    .locals
                    .iter()
                    .find(|fact| fact.key == key)
                    .map(|fact| match &fact.saved {
                        super::super::inputs::SavedAttachmentRef::Connection(text) => {
                            super::super::inputs::SavedAttachmentRef::Connection(text)
                        }
                        super::super::inputs::SavedAttachmentRef::Group(text) => {
                            super::super::inputs::SavedAttachmentRef::Group(text)
                        }
                        super::super::inputs::SavedAttachmentRef::LocalTerminal(text) => {
                            super::super::inputs::SavedAttachmentRef::LocalTerminal(text)
                        }
                    }),
            };
            if let Some(fact) = fact {
                let (text, allocation) = super::copied(
                    ledger,
                    match fact {
                        super::super::inputs::SavedAttachmentRef::Connection(text)
                        | super::super::inputs::SavedAttachmentRef::Group(text)
                        | super::super::inputs::SavedAttachmentRef::LocalTerminal(text) => text,
                    },
                )?
                .into_parts();
                let value = match fact {
                    super::super::inputs::SavedAttachmentRef::Connection(_) => {
                        SavedAttachment::Connection(text)
                    }
                    super::super::inputs::SavedAttachmentRef::Group(_) => {
                        SavedAttachment::Group(text)
                    }
                    super::super::inputs::SavedAttachmentRef::LocalTerminal(_) => {
                        SavedAttachment::LocalTerminal(text)
                    }
                };
                let value = NativeOwned::from_parts(value, allocation);
                if !saved.contains(value.get()) {
                    saved.push(value)?;
                }
            }
        }
        Ok(())
    }
}
struct CurrentAttachments<'a> {
    live: Option<&'a LiveInputs>,
    saved: &'a [AttachmentSlot],
}
enum CurrentAttachment<'a> {
    Live(&'a LiveAttachmentSlot),
    Saved(&'a AttachmentSlot),
}
impl CurrentAttachments<'_> {
    fn len(&self) -> usize {
        self.live
            .map_or(self.saved.len(), |live| live.attachments().len())
    }
    fn iter(&self) -> CurrentAttachmentIter<'_> {
        match self.live {
            Some(live) => CurrentAttachmentIter::Live(live.attachments().iter()),
            None => CurrentAttachmentIter::Saved(self.saved.iter()),
        }
    }
    fn position(&self, value: LiveAttachmentRef<'_>) -> Option<usize> {
        self.iter().position(|item| match item {
            CurrentAttachment::Live(left) => match (left, value) {
                (LiveAttachmentSlot::Connection(left), LiveAttachmentRef::Connection(right))
                | (LiveAttachmentSlot::Group(left), LiveAttachmentRef::Group(right))
                | (
                    LiveAttachmentSlot::UnavailableLocal(left),
                    LiveAttachmentRef::UnavailableLocal(right),
                ) => &*left.read() == right,
                (LiveAttachmentSlot::LocalItem(left), LiveAttachmentRef::LocalItem(right)) => {
                    *left == right
                }
                _ => false,
            },
            CurrentAttachment::Saved(left) => match (left, value) {
                (AttachmentSlot::Connection(left), LiveAttachmentRef::Connection(right))
                | (AttachmentSlot::Group(left), LiveAttachmentRef::Group(right))
                | (
                    AttachmentSlot::LocalTerminal(left),
                    LiveAttachmentRef::UnavailableLocal(right),
                ) => &*left.read() == right,
                _ => false,
            },
        })
    }
}

enum CurrentAttachmentIter<'a> {
    Live(std::slice::Iter<'a, LiveAttachmentSlot>),
    Saved(std::slice::Iter<'a, AttachmentSlot>),
}
impl<'a> Iterator for CurrentAttachmentIter<'a> {
    type Item = CurrentAttachment<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Live(values) => values.next().map(CurrentAttachment::Live),
            Self::Saved(values) => values.next().map(CurrentAttachment::Saved),
        }
    }
}
