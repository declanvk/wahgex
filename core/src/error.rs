//! This module contains types and functions related to public-facing errors.

use std::{alloc::LayoutError, error::Error, fmt};
use wasmparser::ExternalKind;

/// Represents an error that can occur during the regex compilation process.
///
/// This error type encapsulates various kinds of issues, from NFA construction
/// problems to memory layout errors and unsupported regex features.
#[derive(Debug)]
pub struct BuildError {
    kind: Box<BuildErrorKind>,
}

impl BuildError {
    pub(crate) fn missing_export(name: impl Into<String>) -> Self {
        Self {
            kind: Box::new(BuildErrorKind::MissingExport(name.into())),
        }
    }

    pub(crate) fn incorrect_export_type(
        name: impl Into<String>,
        expected: ExternalKind,
        found: ExternalKind,
    ) -> Self {
        Self {
            kind: Box::new(BuildErrorKind::IncorrectExportType {
                name: name.into(),
                expected,
                found,
            }),
        }
    }
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &*self.kind {
            BuildErrorKind::Layout(err) => err.fmt(f),
            BuildErrorKind::NFABuild(err) => err.fmt(f),
            BuildErrorKind::LookaroundUnicode(err) => err.fmt(f),
            BuildErrorKind::WasmBytesValidationError(err) => err.fmt(f),
            BuildErrorKind::MissingExport(name) => write!(f, "missing required export `{name}`"),
            BuildErrorKind::IncorrectExportType {
                name,
                expected,
                found,
            } => {
                write!(
                    f,
                    "incorrect export type for `{name}`: expected {expected:?}, found {found:?}",
                )
            },
        }
    }
}

impl Error for BuildError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &*self.kind {
            BuildErrorKind::Layout(err) => Some(err),
            BuildErrorKind::NFABuild(err) => Some(err),
            BuildErrorKind::LookaroundUnicode(err) => Some(err),
            BuildErrorKind::WasmBytesValidationError(err) => Some(err),
            _ => None,
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

impl From<wasmparser::BinaryReaderError> for BuildError {
    fn from(value: wasmparser::BinaryReaderError) -> Self {
        Self {
            kind: Box::new(BuildErrorKind::WasmBytesValidationError(value)),
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
    WasmBytesValidationError(wasmparser::BinaryReaderError),
    MissingExport(String),
    IncorrectExportType {
        name: String,
        expected: ExternalKind,
        found: ExternalKind,
    },
}
