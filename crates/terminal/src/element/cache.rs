//! Shared paint snapshots, with one owner for invalidation and publication.
use super::Geometry;
use crate::highlighting::Highlights;
use nocterm_vt::{Emulator, Frame, Palette};
use std::{
    cell::{Cell, Ref, RefCell},
    rc::Rc,
};

#[derive(Clone)]
pub(crate) struct FrameCache {
    frame: Rc<RefCell<Frame>>,
    dirty: Rc<Cell<bool>>,
    version: Rc<Cell<Option<(u64, u64)>>>,
    highlights: Rc<RefCell<Highlights>>,
    geometry: Rc<Cell<Option<Geometry>>>,
    palette: Rc<Cell<Option<Palette>>>,
}

impl Default for FrameCache {
    fn default() -> Self {
        Self {
            frame: Rc::default(),
            dirty: Rc::new(Cell::new(true)),
            version: Rc::default(),
            highlights: Rc::default(),
            geometry: Rc::default(),
            palette: Rc::default(),
        }
    }
}

impl FrameCache {
    pub(crate) fn invalidate(&self) {
        self.dirty.set(true);
    }
    pub(crate) fn frame(&self) -> Ref<'_, Frame> {
        self.frame.borrow()
    }
    pub(crate) fn highlights(&self) -> Ref<'_, Highlights> {
        self.highlights.borrow()
    }
    pub(crate) fn geometry(&self) -> Option<Geometry> {
        self.geometry.get()
    }
    pub(crate) fn publish_geometry(&self, geometry: Geometry) {
        self.geometry.set(Some(geometry));
    }
    pub(crate) fn clear_highlights(&self) {
        self.highlights.borrow_mut().clear();
    }
    pub(crate) fn update_highlights(&self, generation: u64, enabled: bool) {
        self.highlights
            .borrow_mut()
            .update(&self.frame.borrow(), generation, enabled);
    }
    pub(crate) fn apply_palette(&self, palette: Palette) -> bool {
        if self.palette.get() == Some(palette) {
            return false;
        }
        self.palette.set(Some(palette));
        self.invalidate();
        true
    }
    pub(crate) fn publish(&self, emulator: &Emulator, revision: u64) -> bool {
        let version = (emulator.generation(), revision);
        if !self.dirty.get()
            && self.version.get() == Some(version)
            && self.frame.borrow().size == emulator.size()
        {
            return false;
        }
        emulator.snapshot(&mut self.frame.borrow_mut());
        self.version.set(Some(version));
        self.dirty.set(false);
        true
    }
}
