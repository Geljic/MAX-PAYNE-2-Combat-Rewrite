//! `payne-extract` library: readers for Max Payne 2's skeleton, animation and
//! skin formats. Nothing here depends on the game being installed; the
//! formats are documented in `docs/formats/`.

pub mod bake;
pub mod database;
pub mod generate;
pub mod kf2;
pub mod kfs;
pub mod measure;
pub mod reader;
pub mod skd;
pub mod xform;

#[cfg(test)]
mod testutil;
