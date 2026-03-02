//! `wahgex` is a library for compiling regular expressions into
//! WebAssembly modules that can be executed efficiently.

#![deny(missing_docs, missing_debug_implementations)]
#![warn(missing_debug_implementations)]

use std::borrow::Cow;

#[cfg(feature = "compile")]
use compile::compile_from_nfa;
use regex_automata::nfa::thompson::Compiler;
use wasmparser::{ExternalKind, Parser, Payload};

pub use crate::error::BuildError;
pub use regex_automata::{
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
    include_names: Option<bool>,
    compact_data_section: Option<bool>,
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

    /// Configures whether the output WASM module will contain a [name section]
    /// identifying the module components.
    ///
    /// This value defaults to `false`, setting this to `true` will make it
    /// easier to introspect the module.
    ///
    /// [name section]: https://webassembly.github.io/spec/core/appendix/custom.html#name-section
    pub fn include_names(mut self, include_names: bool) -> Self {
        self.include_names = Some(include_names);
        self
    }

    /// Return `true` if the [name section] will be included in the output WASM
    /// module.
    ///
    /// [name section]: https://webassembly.github.io/spec/core/appendix/custom.html#name-section
    pub fn get_include_names(&self) -> bool {
        self.include_names.unwrap_or(false)
    }

    /// Returns the configured memory page size in bytes.
    pub fn get_page_size(&self) -> usize {
        Self::DEFAULT_PAGE_SIZE
    }

    /// Configures whether the output WASM module will have adjacent data
    /// segments compacted into a single segment.
    ///
    /// This value defaults to `false`, setting this to `true` can make the WASM
    /// module smaller in cases where there are many states in the regex.
    pub fn compact_data_section(mut self, compact_data_section: bool) -> Self {
        self.compact_data_section = Some(compact_data_section);
        self
    }

    /// Return `true` if the WASM module data section will be compacted.
    pub fn get_compact_data_section(&self) -> bool {
        self.compact_data_section.unwrap_or(true)
    }

    /// Overwrites the current configuration with options from another config.
    ///
    /// Options set in `other` take precedence over options in `self`.
    fn overwrite(self, other: Self) -> Self {
        Self {
            #[cfg(test)]
            export_state: other.export_state.or(self.export_state),
            #[cfg(test)]
            export_all_functions: other.export_all_functions.or(self.export_all_functions),
            include_names: other.include_names.or(self.include_names),
            compact_data_section: other.compact_data_section.or(self.compact_data_section),
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

    /// Return the current [`Config`] setting.
    pub fn get_config(&self) -> &Config {
        &self.config
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
    pub const fn from_static_bytes_unchecked(bytes: &'static [u8]) -> Self {
        Self {
            bytes: Cow::Borrowed(bytes),
        }
    }

    /// Creates a `RegexBytecode` instance from a byte slice after validating
    /// that it is a valid WebAssembly module with the expected shape.
    ///
    /// # Errors
    ///  - If the provided bytes are not a validate WASM module
    ///  - If the provided bytes are a WASM module, but don't have the expected
    ///    exports.
    ///
    /// To be clear, the validation on this function won't be able to detect if
    /// the passed WASM module was internally tampered with, so this should
    /// never be used with anything other than bytecode that was produced by
    /// this library.
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Result<Self, BuildError> {
        let bytes = bytes.into();
        wasmparser::validate(&bytes)?;
        Self::validate_module_exports(&bytes)?;

        Ok(Self::from_bytes_unchecked(bytes))
    }

    /// Creates a `RegexBytecode` instance from a static byte slice after
    /// validating that it is a valid WebAssembly module with the expected
    /// shape.
    ///
    ///
    /// # Errors
    ///  - If the provided bytes are not a validate WASM module
    ///  - If the provided bytes are a WASM module, but don't have the expected
    ///    exports.
    ///
    /// To be clear, the validation on this function won't be able to detect if
    /// the passed WASM module was internally tampered with, so this should
    /// never be used with anything other than bytecode that was produced by
    /// this library.
    pub fn from_static_bytes(bytes: &'static [u8]) -> Result<Self, BuildError> {
        wasmparser::validate(bytes)?;
        Self::validate_module_exports(bytes)?;

        Ok(Self::from_static_bytes_unchecked(bytes))
    }

    /// Returns reference to the bytecode.
    pub const fn as_ref(&self) -> &[u8] {
        match &self.bytes {
            Cow::Borrowed(bytes) => bytes,
            Cow::Owned(bytes) => bytes.as_slice(),
        }
    }

    fn validate_module_exports(bytes: &[u8]) -> Result<(), BuildError> {
        let mut has_prepare_input = false;
        let mut has_is_match = false;
        let mut has_haystack = false;

        for payload in Parser::new(0).parse_all(bytes) {
            if let Payload::ExportSection(reader) = payload? {
                for export in reader {
                    let export = export?;
                    match export.name {
                        "prepare_input" => {
                            if let ExternalKind::Func = export.kind {
                                has_prepare_input = true;
                            } else {
                                return Err(BuildError::incorrect_export_type(
                                    "prepare_input",
                                    ExternalKind::Func,
                                    export.kind,
                                ));
                            }
                        },
                        "is_match" => {
                            if let ExternalKind::Func = export.kind {
                                has_is_match = true;
                            } else {
                                return Err(BuildError::incorrect_export_type(
                                    "is_match",
                                    ExternalKind::Func,
                                    export.kind,
                                ));
                            }
                        },
                        "haystack" => {
                            if let ExternalKind::Memory = export.kind {
                                has_haystack = true;
                            } else {
                                return Err(BuildError::incorrect_export_type(
                                    "haystack",
                                    ExternalKind::Memory,
                                    export.kind,
                                ));
                            }
                        },
                        _ => {},
                    }
                }
            }
        }

        if !has_prepare_input {
            return Err(BuildError::missing_export("prepare_input"));
        }

        if !has_is_match {
            return Err(BuildError::missing_export("is_match"));
        }

        if !has_haystack {
            return Err(BuildError::missing_export("haystack"));
        }

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

#[cfg(test)]
mod tests {
    use super::*;
    use wasmparser::ExternalKind;

    #[test]
    fn test_from_bytes_invalid_wasm() {
        let err = RegexBytecode::from_bytes(vec![]).unwrap_err();
        assert!(err.to_string().contains("unexpected end-of-file"), "{err}");
    }

    #[test]
    fn test_from_bytes_incorrect_export_type() {
        // Module with 'prepare_input' as a Memory instead of Func.
        let mut module = wasm_encoder::Module::new();

        let mut mems = wasm_encoder::MemorySection::new();
        mems.memory(wasm_encoder::MemoryType {
            minimum: 1,
            maximum: None,
            memory64: false,
            shared: false,
            page_size_log2: None,
        });
        module.section(&mems);

        let mut exports = wasm_encoder::ExportSection::new();
        exports.export("prepare_input", wasm_encoder::ExportKind::Memory, 0);
        module.section(&exports);
        let wasm = module.finish();

        let err = RegexBytecode::from_bytes(wasm).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!(
                "incorrect export type for `prepare_input`: expected {:?}, found {:?}",
                ExternalKind::Func,
                ExternalKind::Memory
            )
        );
    }

    #[test]
    fn test_from_bytes_missing_is_match() {
        let mut module = wasm_encoder::Module::new();

        let mut types = wasm_encoder::TypeSection::new();
        types.ty().function([], []);
        module.section(&types);

        let mut funcs = wasm_encoder::FunctionSection::new();
        funcs.function(0);
        module.section(&funcs);

        let mut exports = wasm_encoder::ExportSection::new();
        exports.export("prepare_input", wasm_encoder::ExportKind::Func, 0);
        module.section(&exports);

        let mut code = wasm_encoder::CodeSection::new();
        let mut func = wasm_encoder::Function::new([]);
        func.instruction(&wasm_encoder::Instruction::End);
        code.function(&func);
        module.section(&code);

        let wasm = module.finish();

        let err = RegexBytecode::from_bytes(wasm).unwrap_err();
        assert_eq!(err.to_string(), "missing required export `is_match`");
    }

    #[test]
    fn test_from_bytes_missing_haystack() {
        let mut module = wasm_encoder::Module::new();

        let mut types = wasm_encoder::TypeSection::new();
        types.ty().function([], []);
        module.section(&types);

        let mut funcs = wasm_encoder::FunctionSection::new();
        funcs.function(0);
        funcs.function(0);
        module.section(&funcs);

        let mut exports = wasm_encoder::ExportSection::new();
        exports.export("prepare_input", wasm_encoder::ExportKind::Func, 0);
        exports.export("is_match", wasm_encoder::ExportKind::Func, 1);
        module.section(&exports);

        let mut code = wasm_encoder::CodeSection::new();
        let mut func = wasm_encoder::Function::new([]);
        func.instruction(&wasm_encoder::Instruction::End);
        code.function(&func);
        code.function(&func);
        module.section(&code);

        let wasm = module.finish();

        let err = RegexBytecode::from_bytes(wasm).unwrap_err();
        assert_eq!(err.to_string(), "missing required export `haystack`");
    }
}
