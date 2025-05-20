//! TODO: Write docs for this crate

#![deny(missing_docs, missing_debug_implementations)]

use compile::{compile_from_nfa, CompiledRegex};

pub use crate::{
    compile::input::{InputOpts, PrepareInputResult},
    error::BuildError,
};

mod compile;
mod error;
mod runtime;
mod util;

/// TODO: Write docs for this item
#[derive(Debug, Clone, Copy, Default)]
pub struct Config {
    #[cfg(test)]
    export_state: Option<bool>,
    #[cfg(test)]
    export_all_functions: Option<bool>,
}

impl Config {
    /// TODO: Write docs for this item
    pub const DEFAULT_PAGE_SIZE: usize = 64 * 1024;

    /// TODO: Write docs for this item
    pub fn new() -> Self {
        Self::default()
    }

    /// TODO: Write docs for this item
    #[cfg(test)]
    pub fn export_state(mut self, export_state: bool) -> Self {
        self.export_state = Some(export_state);
        self
    }

    /// TODO: Write docs for this item
    #[cfg(test)]
    pub fn get_export_state(&self) -> bool {
        self.export_state.unwrap_or(false)
    }

    /// TODO: Write docs for this item
    #[cfg(test)]
    pub fn export_all_functions(mut self, export_all_functions: bool) -> Self {
        self.export_all_functions = Some(export_all_functions);
        self
    }

    /// TODO: Write docs for this item
    #[cfg(test)]
    pub fn get_export_all_functions(&self) -> bool {
        self.export_all_functions.unwrap_or(false)
    }

    /// TODO: Write docs for this item
    pub fn get_page_size(&self) -> usize {
        Self::DEFAULT_PAGE_SIZE
    }

    /// TODO: Write docs for this item
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

/// TODO: Write docs for this item
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
    /// Create a new PikeVM builder with its default configuration.
    pub fn new() -> Builder {
        Self::default()
    }

    /// TODO: Write docs for this item
    pub fn build(&self, pattern: &str) -> Result<PikeVM, BuildError> {
        self.build_many(&[pattern])
    }

    /// TODO: Write docs for this item
    pub fn build_many<P: AsRef<str>>(&self, patterns: &[P]) -> Result<PikeVM, BuildError> {
        let nfa = self.thompson.build_many(patterns)?;
        self.build_from_nfa(nfa)
    }

    /// TODO: Write docs for this item
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

    /// TODO: Write docs for this item
    pub fn configure(&mut self, config: Config) -> &mut Builder {
        self.config = self.config.overwrite(config);
        self
    }

    /// TODO: Write docs for this item
    pub fn syntax(&mut self, config: regex_automata::util::syntax::Config) -> &mut Builder {
        self.thompson.syntax(config);
        self
    }

    /// TODO: Write docs for this item
    pub fn thompson(&mut self, config: regex_automata::nfa::thompson::Config) -> &mut Builder {
        self.thompson.configure(config);
        self
    }
}

/// TODO: Write docs for this item
// TODO: Needs a better name
#[derive(Debug)]
pub struct PikeVM {
    config: Config,
    nfa: regex_automata::nfa::thompson::NFA,
    wasm: CompiledRegex,
}

impl PikeVM {
    /// TODO: Write docs for this item
    pub fn new(pattern: &str) -> Result<PikeVM, BuildError> {
        PikeVM::builder().build(pattern)
    }

    /// TODO: Write docs for this item
    pub fn new_many<P: AsRef<str>>(patterns: &[P]) -> Result<PikeVM, BuildError> {
        PikeVM::builder().build_many(patterns)
    }

    /// TODO: Write docs for this item
    pub fn new_from_nfa(nfa: regex_automata::nfa::thompson::NFA) -> Result<PikeVM, BuildError> {
        PikeVM::builder().build_from_nfa(nfa)
    }

    /// TODO: Write docs for this item
    pub fn always_match() -> Result<PikeVM, BuildError> {
        let nfa = regex_automata::nfa::thompson::NFA::always_match();
        PikeVM::new_from_nfa(nfa)
    }

    /// TODO: Write docs for this item
    pub fn never_match() -> Result<PikeVM, BuildError> {
        let nfa = regex_automata::nfa::thompson::NFA::never_match();
        PikeVM::new_from_nfa(nfa)
    }

    /// TODO: Write docs for this item
    pub fn config() -> Config {
        Config::new()
    }

    /// TODO: Write docs for this item
    pub fn builder() -> Builder {
        Builder::new()
    }

    /// TODO: Write docs for this item
    pub fn pattern_len(&self) -> usize {
        self.nfa.pattern_len()
    }

    /// Return the config for this `PikeVM`.
    #[inline]
    pub fn get_config(&self) -> &Config {
        &self.config
    }

    /// Returns a reference to the underlying NFA.
    #[inline]
    pub fn get_nfa(&self) -> &regex_automata::nfa::thompson::NFA {
        &self.nfa
    }

    /// Returns a reference to the compiled WASM bytes.
    #[inline]
    pub fn get_wasm(&self) -> &[u8] {
        self.wasm.as_ref()
    }
}

impl PikeVM {
    // TODO: Need to implement `is_match`, `find`, `captures`, `find_iter`,
    // `captures_iter`
}
