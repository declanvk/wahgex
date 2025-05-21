//! This module contains types and functions related to public-facing errors.

use std::{alloc::LayoutError, error::Error, fmt};

/// TODO: Write docs for this item
#[derive(Debug)]
pub struct BuildError {
    kind: Box<BuildErrorKind>,
}

impl BuildError {
    pub(crate) fn unsupported(feature: impl Into<String>) -> Self {
        Self {
            kind: Box::new(BuildErrorKind::Unsupported(feature.into())),
        }
    }

    /// Return true if the error was caused by an unsupported feature.
    pub fn is_unsupported(&self) -> bool {
        matches!(&*self.kind, BuildErrorKind::Unsupported(_))
    }
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &*self.kind {
            BuildErrorKind::Layout(err) => err.fmt(f),
            BuildErrorKind::NFABuild(err) => err.fmt(f),
            BuildErrorKind::LookaroundUnicode(err) => err.fmt(f),
            BuildErrorKind::Unsupported(feature) => write!(f, "Unsupported feature: {feature}"),
        }
    }
}

impl Error for BuildError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &*self.kind {
            BuildErrorKind::Layout(err) => Some(err),
            BuildErrorKind::NFABuild(err) => Some(err),
            BuildErrorKind::LookaroundUnicode(err) => Some(err),
            BuildErrorKind::Unsupported(_) => None,
        }
    }
}

impl From<LayoutError> for BuildError {
    fn from(value: LayoutError) -> Self {
        Self {
            kind: Box::new(BuildErrorKind::Layout(value)),
        }
    }
}

impl From<regex_automata::nfa::thompson::BuildError> for BuildError {
    fn from(value: regex_automata::nfa::thompson::BuildError) -> Self {
        Self {
            kind: Box::new(BuildErrorKind::NFABuild(value)),
        }
    }
}

impl From<regex_automata::util::look::UnicodeWordBoundaryError> for BuildError {
    fn from(value: regex_automata::util::look::UnicodeWordBoundaryError) -> Self {
        Self {
            kind: Box::new(BuildErrorKind::LookaroundUnicode(value)),
        }
    }
}

/// TODO: Write docs for this item
#[derive(Debug)]
enum BuildErrorKind {
    Layout(LayoutError),
    NFABuild(regex_automata::nfa::thompson::BuildError),
    LookaroundUnicode(regex_automata::util::look::UnicodeWordBoundaryError),
    Unsupported(String),
}
