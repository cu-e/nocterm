//! Renders validated user plugin views inside a host frame, as the application draws its own.
//!
//! A plugin returns a [`View`](nocterm_plugin_api::View) of plain nodes. [`PluginView`] checks it
//! again and draws it with the colours of a [`ViewStyle`]. The components of the contract
//! (section, item, icon button, caption, field, and a chart drawn as the application draws its
//! graphs) are drawn with the measures of the application's own panels, so a plugin needs no
//! styling to fit in. The frame is the host's: a bar item and a toolbar (a strip across, a rail
//! along a side, in small text) lead with a marker, a panel's provenance is written by the shell beside its title
//! ([`PluginView::label`]), and a view the host refuses becomes a fixed-width fault chip. After
//! `mark_dev`, for a developer session, the label ends in " · DEV" and a bar item carries a DEV
//! chip. A control reports an [`Interaction`] to the callback that the application supplied;
//! folding a section is the user's and is kept here, without asking the plugin.
//!
//! Owns: node layout and the component measures (a code node in the monospace font of the
//! supplied hooks, the painting of a chart), the mapping of tones onto the supplied `ViewStyle`, pointer and
//! focused-control gestures, the folds of sections, the frame, the marker and the fault chip.
//! Does not own: grants, effects, panels, theme lookup, icons (supplied as an [`IconFn`]),
//! tooltips ([`TooltipFn`]), redrawing ([`RedrawFn`]), focus handles (supplied as a
//! [`FocusFn`]), settings, any I/O.
//! Ports used: none. Settings: none. Globals: none.
//! Invariants: under the xtask isolation rules (I1–I5); it draws only validated views; it never
//! moves focus itself and never tells the plugin which key was pressed; the caller redraws the
//! view after `show` or `fault`, because this crate does not notify.

mod chart;
mod components;
mod frame;
mod paint;
mod style;
mod tree;
mod view;

pub use frame::FrameKind;
pub use style::ViewStyle;
pub use view::{
    EventFn, FocusFn, Hooks, IconFn, Interaction, Origin, PluginView, RedrawFn, TooltipFn,
};

#[cfg(test)]
mod tests;
