//! `wahgex-core` is a library for compiling regular expressions into
//! WebAssembly modules that can be executed efficiently.

#![deny(missing_docs, missing_debug_implementations)]
#![warn(missing_debug_implementations)]

use compile::{compile_from_nfa, CompiledRegex};

pub use crate::{
    compile::input::{InputOpts, PrepareInputResult},
    error::BuildError,
};

mod compile;
mod error;
mod runtime;
mod util;

/// Configuration options for building a [`PikeVM`].
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

/// A builder for compiling regular expressions into a [`PikeVM`].
#[derive(Clone, Debug)]
pub struct Builder {
    config: Config,
    thompson: regex_automata::nfa::thompson::Compiler,
}

impl Default for Builder {
    fn default() -> Self {
        let default_nfa_config = regex_automata::nfa::thompson::Config::new().shrink(false);
        let mut thompson = regex_automata::nfa::thompson::Compiler::new();
        thompson.configure(default_nfa_config);

        Builder {
            config: Config::default(),
            thompson,
        }
    }
}

impl Builder {
    /// Creates a new PikeVM builder with its default configuration.
    pub fn new() -> Builder {
        Self::default()
    }

    /// Compiles a single regular expression pattern into a [`PikeVM`].
    pub fn build(&self, pattern: &str) -> Result<PikeVM, BuildError> {
        self.build_many(&[pattern])
    }

    /// Compiles multiple regular expression patterns into a single [`PikeVM`].
    pub fn build_many<P: AsRef<str>>(&self, patterns: &[P]) -> Result<PikeVM, BuildError> {
        let nfa = self.thompson.build_many(patterns)?;
        self.build_from_nfa(nfa)
    }

    /// Compiles a Thompson NFA into a [`PikeVM`].
    pub fn build_from_nfa(
        &self,
        nfa: regex_automata::nfa::thompson::NFA,
    ) -> Result<PikeVM, BuildError> {
        nfa.look_set_any().available()?;
        let wasm = compile_from_nfa(nfa.clone(), self.config)?;
        Ok(PikeVM {
            config: self.config,
            nfa,
            wasm,
        })
    }

    /// Configures the builder with the given [`Config`].
    pub fn configure(&mut self, config: Config) -> &mut Builder {
        self.config = self.config.overwrite(config);
        self
    }

    /// Configures the syntax options for the underlying regex compiler.
    pub fn syntax(&mut self, config: regex_automata::util::syntax::Config) -> &mut Builder {
        self.thompson.syntax(config);
        self
    }

    /// Configures the Thompson NFA compiler options.
    pub fn thompson(&mut self, config: regex_automata::nfa::thompson::Config) -> &mut Builder {
        self.thompson.configure(config);
        self
    }
}

/// A compiled regular expression represented as a Pike VM, ready for matching.
#[derive(Debug)]
pub struct PikeVM {
    config: Config,
    nfa: regex_automata::nfa::thompson::NFA,
    wasm: CompiledRegex,
}

impl PikeVM {
    /// Compiles a single regular expression pattern into a new [`PikeVM`] using
    /// the default builder.
    pub fn new(pattern: &str) -> Result<PikeVM, BuildError> {
        PikeVM::builder().build(pattern)
    }

    /// Compiles multiple regular expression patterns into a single new
    /// [`PikeVM`] using the default builder.
    pub fn new_many<P: AsRef<str>>(patterns: &[P]) -> Result<PikeVM, BuildError> {
        PikeVM::builder().build_many(patterns)
    }

    /// Creates a new [`PikeVM`] directly from a Thompson NFA using the default
    /// builder.
    pub fn new_from_nfa(nfa: regex_automata::nfa::thompson::NFA) -> Result<PikeVM, BuildError> {
        PikeVM::builder().build_from_nfa(nfa)
    }

    /// Creates a [`PikeVM`] that always matches the empty string at any
    /// position.
    pub fn always_match() -> Result<PikeVM, BuildError> {
        let nfa = regex_automata::nfa::thompson::NFA::always_match();
        PikeVM::new_from_nfa(nfa)
    }

    /// Creates a [`PikeVM`] that never matches.
    pub fn never_match() -> Result<PikeVM, BuildError> {
        let nfa = regex_automata::nfa::thompson::NFA::never_match();
        PikeVM::new_from_nfa(nfa)
    }

    /// Returns a new default [`Config`] for configuring a [`Builder`].
    pub fn config() -> Config {
        Config::new()
    }

    /// Returns a new default [`Builder`] for compiling regular expressions.
    pub fn builder() -> Builder {
        Builder::new()
    }

    /// Returns the number of patterns compiled into this PikeVM.
    pub fn pattern_len(&self) -> usize {
        self.nfa.pattern_len()
    }

    /// Return the config for this `PikeVM`.
    ///
    /// Note that this is the configuration used to *build* the PikeVM,
    /// not necessarily the configuration used for a specific match operation.
    #[inline]
    pub fn get_config(&self) -> &Config {
        &self.config
    }

    /// Returns a reference to the underlying NFA.
    ///
    /// This is the NFA that was compiled into the PikeVM.
    #[inline]
    pub fn get_nfa(&self) -> &regex_automata::nfa::thompson::NFA {
        &self.nfa
    }

    /// Returns a reference to the compiled WASM bytes.
    ///
    /// These bytes represent the compiled PikeVM logic.
    #[inline]
    pub fn get_wasm(&self) -> &[u8] {
        self.wasm.as_ref()
    }
}

impl PikeVM {
    // TODO: Need to implement `is_match`, `find`, `captures`, `find_iter`,
    // `captures_iter`
}
