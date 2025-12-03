//! This module contains types and functions related to public-facing errors.

use std::{alloc::LayoutError, error::Error, fmt};

/// Represents an error that can occur during the regex compilation process.
///
/// This error type encapsulates various kinds of issues, from NFA construction
/// problems to memory layout errors and unsupported regex features.
#[derive(Debug)]
pub struct BuildError {
    kind: Box<BuildErrorKind>,
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &*self.kind {
            BuildErrorKind::Layout(err) => err.fmt(f),
            BuildErrorKind::NFABuild(err) => err.fmt(f),
            BuildErrorKind::LookaroundUnicode(err) => err.fmt(f),
        }
    }
}

impl Error for BuildError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &*self.kind {
            BuildErrorKind::Layout(err) => Some(err),
            BuildErrorKind::NFABuild(err) => Some(err),
            BuildErrorKind::LookaroundUnicode(err) => Some(err),
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

/// Represents the specific kind of a [`BuildError`].
///
/// This enum provides more granular information about the underlying cause of a
/// `BuildError`.
#[derive(Debug)]
enum BuildErrorKind {
    Layout(LayoutError),
    NFABuild(regex_automata::nfa::thompson::BuildError),
    LookaroundUnicode(regex_automata::util::look::UnicodeWordBoundaryError),
}
