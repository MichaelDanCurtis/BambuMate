//! Reading 3D models (STL and 3MF) for the viewport, and working out where
//! they may print badly (spec 2026-10-22). Read-only: files are never
//! written.

pub mod mesh;
pub mod stl;
pub mod threemf;

/// Largest model file the viewport will read.
pub const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
/// Largest mesh the viewport will build.
pub const MAX_TRIANGLES: usize = 5_000_000;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ModelError {
    #[error("This file isn't an STL or 3MF.")]
    NotAModel,
    #[error("The file is too large to preview ({0} MB; the limit is 256 MB).")]
    FileTooLarge(u64),
    #[error("The model is too large to preview ({0} triangles; the limit is 5,000,000).")]
    TooManyTriangles(usize),
    #[error("The model has no triangles.")]
    Empty,
    #[error("The STL file is cut short.")]
    Truncated,
    #[error("The 3MF has no printable objects.")]
    NoObjects,
    #[error("The 3MF couldn't be read: {0}")]
    Archive(String),
    #[error("The model couldn't be read: {0}")]
    Io(String),
}
