//! TuneSync's settings window as a portable view model (docs/settings-onepage.html):
//! [`model::Model`] is what it shows, [`view::layout`] makes a [`view::Frame`] of
//! [`draw::Item`]s and tap regions, [`view::View`] turns pointer events into
//! [`view::Command`]s. The Windows shell paints frames with Direct2D; [`svg`] renders
//! them here for previews and tests.
pub mod draw;
pub mod model;
pub mod svg;
pub mod view;
