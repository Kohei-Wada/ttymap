//! Low-level 2D geometry primitives used by [`super::canvas`].
//!
//! Everything here is a pure function of its inputs — no Canvas state,
//! no drawing side effects — so the algorithms can be unit-tested in
//! isolation.

pub(crate) mod bresenham;
pub(crate) mod clip;

pub(crate) use bresenham::BresenhamIter;
pub(crate) use clip::{clip_line, sutherland_hodgman_into};
