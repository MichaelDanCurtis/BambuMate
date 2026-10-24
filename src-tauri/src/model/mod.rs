//! Reading 3D models (STL and 3MF) for the viewport, and working out where
//! they may print badly (spec 2026-10-22). Read-only: files are never
//! written.

pub mod edges;
pub mod mesh;
pub mod risks;
pub mod sample;
pub mod stl;
pub mod threemf;
pub mod view;

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
    /// A plain sentence saying why the 3MF couldn't be read.
    #[error("{0}")]
    Archive(&'static str),
    #[error("The model couldn't be read: {0}")]
    Io(String),
}

impl ModelError {
    /// The sentence the page shows. Fixed text for every variant: `Io` wraps
    /// the operating system's error text, which differs by platform and says
    /// nothing the user can act on, so it stays in the log. The match is
    /// exhaustive on purpose; a new variant must pick its sentence here.
    pub fn user_message(&self) -> String {
        match self {
            Self::NotAModel => "This file isn't an STL or 3MF.".to_string(),
            Self::FileTooLarge(mb) => {
                format!("The file is too large to preview ({mb} MB; the limit is 256 MB).")
            }
            Self::TooManyTriangles(n) => format!(
                "The model is too large to preview ({n} triangles; the limit is 5,000,000)."
            ),
            Self::Empty => "The model has no triangles.".to_string(),
            Self::Truncated => "The STL file is cut short.".to_string(),
            Self::NoObjects => "The 3MF has no printable objects.".to_string(),
            Self::Archive(sentence) => (*sentence).to_string(),
            Self::Io(_) => "The model file couldn't be read.".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_variant() -> Vec<ModelError> {
        vec![
            ModelError::NotAModel,
            ModelError::FileTooLarge(300),
            ModelError::TooManyTriangles(6_000_000),
            ModelError::Empty,
            ModelError::Truncated,
            ModelError::NoObjects,
            ModelError::Archive("The 3MF is damaged and couldn't be read."),
            ModelError::Io("No such file or directory (os error 2)".into()),
            ModelError::Io("The system cannot find the file specified. (os error 2)".into()),
            ModelError::Io("not a regular file".into()),
        ]
    }

    #[test]
    fn user_messages_never_carry_os_text() {
        for e in every_variant() {
            let m = e.user_message();
            for bad in ["os error", "No such file", "cannot find", "regular file"] {
                assert!(!m.contains(bad), "{e:?} leaks {bad:?}: {m}");
            }
            assert!(m.ends_with('.'), "{e:?} is not a sentence: {m}");
        }
    }

    #[test]
    fn io_reads_as_one_fixed_sentence() {
        let a = ModelError::Io("No such file or directory (os error 2)".into());
        let b = ModelError::Io("Access is denied. (os error 5)".into());
        assert_eq!(a.user_message(), "The model file couldn't be read.");
        assert_eq!(a.user_message(), b.user_message());
    }

    #[test]
    fn the_fixed_variants_read_as_their_display_text() {
        for e in every_variant() {
            if !matches!(e, ModelError::Io(_)) {
                assert_eq!(e.user_message(), e.to_string());
            }
        }
    }

    #[test]
    fn size_limits_include_their_numbers() {
        assert!(ModelError::FileTooLarge(300)
            .user_message()
            .contains("300 MB"));
        assert!(ModelError::TooManyTriangles(6_000_000)
            .user_message()
            .contains("6000000"));
    }
}
