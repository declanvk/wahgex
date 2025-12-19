//! `wahgex` is a library for compiling regular expressions into
//! WebAssembly modules that can be executed efficiently.

#![deny(missing_docs, missing_debug_implementations)]
#![warn(missing_debug_implementations)]

use std::borrow::Cow;

#[cfg(feature = "compile")]
use compile::compile_from_nfa;
use regex_automata::nfa::thompson::Compiler;
use wasmparser::types::Types;

pub use crate::error::BuildError;
pub use ::regex_automata::{
    Input,
    nfa::thompson::{Config as RegexNFAConfig, NFA},
    util::syntax::Config as RegexSyntaxConfig,
};

#[cfg(feature = "compile")]
mod compile;
#[cfg(feature = "wasmi")]
pub mod engines;
mod error;
mod input;

/// Configuration options for building a regular expression.
#[derive(Debug, Clone, Copy, Default)]
pub struct Config {
    #[cfg(test)]
    export_state: Option<bool>,
    #[cfg(test)]
    export_all_functions: Option<bool>,
}

impl Config {
    /// The default size of a memory page in bytes (64 KiB).
    pub const DEFAULT_PAGE_SIZE: usize = 64 * 1024;

    /// Creates a new default configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Configures whether the internal state memory should be exported.
    ///
    /// This is primarily for testing and debugging purposes.
    #[cfg(test)]
    pub fn export_state(mut self, export_state: bool) -> Self {
        self.export_state = Some(export_state);
        self
    }

    /// Returns `true` if the internal state memory is configured to be
    /// exported.
    #[cfg(test)]
    pub fn get_export_state(&self) -> bool {
        self.export_state.unwrap_or(false)
    }

    /// Configures whether all internal functions should be exported.
    ///
    /// This is primarily for testing and debugging purposes.
    #[cfg(test)]
    pub fn export_all_functions(mut self, export_all_functions: bool) -> Self {
        self.export_all_functions = Some(export_all_functions);
        self
    }

    /// Returns `true` if all internal functions are configured to be exported.
    #[cfg(test)]
    pub fn get_export_all_functions(&self) -> bool {
        self.export_all_functions.unwrap_or(false)
    }

    /// Returns the configured memory page size in bytes.
    pub fn get_page_size(&self) -> usize {
        Self::DEFAULT_PAGE_SIZE
    }

    /// Overwrites the current configuration with options from another config.
    ///
    /// Options set in `other` take precedence over options in `self`.
    #[cfg_attr(not(test), expect(unused_variables))]
    fn overwrite(self, other: Self) -> Self {
        Self {
            #[cfg(test)]
            export_state: other.export_state.or(self.export_state),
            #[cfg(test)]
            export_all_functions: other.export_all_functions.or(self.export_all_functions),
        }
    }
}

/// A builder for compiling regular expressions into [`RegexBytecode`].
#[derive(Clone, Debug)]
pub struct Builder {
    config: Config,
    thompson: Compiler,
}

impl Default for Builder {
    fn default() -> Self {
        let default_nfa_config = RegexNFAConfig::new().shrink(false);
        let mut thompson = Compiler::new();
        thompson.configure(default_nfa_config);

        Builder {
            config: Config::default(),
            thompson,
        }
    }
}

impl Builder {
    /// Creates a new regular expression builder with its default configuration.
    pub fn new() -> Builder {
        Self::default()
    }

    /// Compiles a single regular expression pattern into a [`RegexBytecode`]
    /// and [`RegexContext`].
    #[cfg(feature = "compile")]
    pub fn build(&self, pattern: &str) -> Result<(RegexBytecode, RegexContext), BuildError> {
        self.build_many(&[pattern])
    }

    /// Compiles multiple regular expression patterns into a single
    /// [`RegexBytecode`] and [`RegexContext`].
    #[cfg(feature = "compile")]
    pub fn build_many<P: AsRef<str>>(
        &self,
        patterns: &[P],
    ) -> Result<(RegexBytecode, RegexContext), BuildError> {
        let nfa = self.thompson.build_many(patterns)?;
        self.build_from_nfa(nfa)
    }

    /// Compiles a Thompson NFA into a [`RegexBytecode`]
    /// and [`RegexContext`].
    #[cfg(feature = "compile")]
    pub fn build_from_nfa(&self, nfa: NFA) -> Result<(RegexBytecode, RegexContext), BuildError> {
        nfa.look_set_any().available()?;
        let compiled = compile_from_nfa(nfa.clone(), self.config)?;
        Ok((
            compiled,
            RegexContext {
                config: self.config,
                nfa,
            },
        ))
    }

    /// Configures the builder with the given [`Config`].
    pub fn configure(&mut self, config: Config) -> &mut Builder {
        self.config = self.config.overwrite(config);
        self
    }

    /// Configures the syntax options for the underlying regex compiler.
    pub fn syntax(&mut self, config: RegexSyntaxConfig) -> &mut Builder {
        self.thompson.syntax(config);
        self
    }

    /// Configures the Thompson NFA compiler options.
    pub fn thompson(&mut self, config: RegexNFAConfig) -> &mut Builder {
        self.thompson.configure(config);
        self
    }
}

/// A compiled regular expression ready for matching.
#[derive(Debug)]
#[non_exhaustive]
pub struct RegexContext {
    /// The configuration used to build the regular expression.
    pub config: Config,
    /// The non-deterministic finite automaton (NFA) used to build the regular
    /// expression.
    pub nfa: NFA,
}

impl RegexContext {
    /// Returns a new default [`Config`] for configuring a [`Builder`].
    pub fn config() -> Config {
        Config::new()
    }

    /// Returns a new default [`Builder`] for compiling regular expressions.
    pub fn builder() -> Builder {
        Builder::new()
    }
}

/// Represents a regular expression that has been compiled into WebAssembly
/// bytes.
#[derive(Debug)]
pub struct RegexBytecode {
    bytes: Cow<'static, [u8]>,
}

impl RegexBytecode {
    /// Creates a `RegexBytecode` instance from a byte slice without performing
    /// any validation.
    ///
    /// This is an unsafe operation that should only be used when the byte slice
    /// is known to be a valid WebAssembly module with the expected shape.
    /// For safe creation, use `from_bytes` instead.
    pub fn from_bytes_unchecked(bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            bytes: bytes.into().into(),
        }
    }

    /// Creates a `RegexBytecode` instance from a static byte slice without
    /// performing any validation.
    ///
    /// This is an unsafe operation that should only be used when the byte slice
    /// is known to be a valid WebAssembly module with the expected shape.
    /// For safe creation, use `from_static_bytes` instead.
    pub const fn from_static_bytes_unchecked(bytes: &'static [u8]) -> Self {
        Self {
            bytes: Cow::Borrowed(bytes),
        }
    }

    /// Creates a `RegexBytecode` instance from a byte slice after validating
    /// that it is a valid WebAssembly module with the expected shape.
    ///
    /// This is the recommended way to create a `RegexBytecode` instance from a
    /// dynamic byte slice.
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Result<Self, BuildError> {
        let bytes = bytes.into();
        let types = wasmparser::validate(&bytes)?;
        Self::validate_module_shape(types)?;

        Ok(Self::from_bytes_unchecked(bytes))
    }

    /// Creates a `RegexBytecode` instance from a static byte slice after
    /// validating that it is a valid WebAssembly module with the expected
    /// shape.
    ///
    /// This is the recommended way to create a `RegexBytecode` instance from a
    /// static byte slice.
    pub fn from_static_bytes(bytes: &'static [u8]) -> Result<Self, BuildError> {
        let types = wasmparser::validate(bytes)?;
        Self::validate_module_shape(types)?;

        Ok(Self::from_static_bytes_unchecked(bytes))
    }

    /// Returns reference to the bytecode.
    pub const fn as_ref(&self) -> &[u8] {
        match &self.bytes {
            Cow::Borrowed(bytes) => bytes,
            Cow::Owned(bytes) => bytes.as_slice(),
        }
    }

    fn validate_module_shape(_types: Types) -> Result<(), BuildError> {
        // TODO: Implement this so that we validate the expected shape of the
        // bytes
        Ok(())
    }
}

impl AsRef<[u8]> for RegexBytecode {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

/// Assert that a given [`Input`] follows some common requirements.
///
/// Namely that:
///  1. The length of the haystack is less than [`usize::MAX`] (I think this
///     condition is impossible to violate since maximum slice length is
///     [`isize::MAX`]).
///  2. The [`input.start()`][Input::end] must be less than or equal to
///     [`input.end`][Input::end].
///  3. The [`input.end()`][Input::end] must be less than or equal to the length
///     of the haystack.
#[cfg(feature = "wasmi")]
fn common_input_validation(input: &Input<'_>) {
    assert!(
        input.haystack().len() < usize::MAX,
        "byte slice lengths must be less than usize MAX",
    );
    let span = input.get_span();
    assert!(
        span.start <= span.end,
        "span start must be less than or equal to span end",
    );
    assert!(
        span.end <= input.haystack().len(),
        "span end must be within bounds of haystack"
    )
}
