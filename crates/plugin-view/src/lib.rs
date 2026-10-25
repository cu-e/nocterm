//! Renders validated user plugin views inside a host frame with a marker and a label.
//!
//! A plugin returns a [`View`](nocterm_plugin_api::View) of plain nodes. [`PluginView`] checks it again, draws it with the
//! colours of a [`ViewStyle`] and wraps it in a frame the plugin cannot style: a marker, the
//! label "Plugin · ⟨name⟩" and, when the view is invalid, a fixed-width fault chip. A button or
//! toggle reports an [`Interaction`] to the callback that the application supplied.
//!
//! Owns: node layout, the mapping of tones onto the supplied `ViewStyle`, pointer and
//! focused-control gestures, the frame, the marker and the fault chip.
//! Does not own: grants, effects, panels, theme lookup, icons (supplied as an [`IconFn`]), focus
//! handles (supplied as a [`FocusFn`]), settings, any I/O.
//! Ports used: none. Settings: none. Globals: none.
//! Invariants: under the xtask isolation rules (I1–I5); it draws only validated views; it never
//! moves focus itself and never tells the plugin which key was pressed; the caller redraws the
//! view after `show` or `fault`, because this crate does not notify.

mod frame;
mod paint;
mod style;
mod tree;
mod view;

pub use frame::FrameKind;
pub use style::ViewStyle;
pub use view::{EventFn, FocusFn, Hooks, IconFn, Interaction, Origin, PluginView};

#[cfg(test)]
mod tests;
