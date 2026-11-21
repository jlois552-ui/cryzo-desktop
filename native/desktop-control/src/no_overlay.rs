//! Outside Windows there is no on-screen overlay yet; the browser tools call these and nothing
//! shows.

use crate::common::Rect;

pub(crate) struct Overlay;

impl Overlay {
    pub(crate) fn step(&self, _step: impl Into<String>) {}

    pub(crate) fn point(&self, _step: impl Into<String>, _rect: Rect) {}

    pub(crate) fn click(&self) {}
}
