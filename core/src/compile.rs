//! This module is responsible for compiling a Thompson NFA (Non-deterministic
//! Finite Automaton) into a WebAssembly module.

use input::{InputFunctions, InputLayout};
use matching::MatchingFunctions;
use state::{StateFunctions, StateLayout};

use crate::RegexBytecode;
pub use crate::error::BuildError;

use self::context::CompileContext;

mod context;
mod epsilon_closure;
pub mod input;
mod instructions;
mod lookaround;
mod matching;
mod pattern;
mod sparse_set;
mod state;
mod transition;
mod util;

/// Compiles a given Thompson NFA into a [`RegexBytecode`] WebAssembly module,
/// using the provided configuration.
pub fn compile_from_nfa(
    nfa: regex_automata::nfa::thompson::NFA,
    config: super::Config,
) -> Result<RegexBytecode, BuildError> {
    let mut ctx = CompileContext::new(nfa, config);
    let state_layout = StateLayout::new(&mut ctx)?;
    let state_funcs = StateFunctions::new(&mut ctx, &state_layout)?;
    let input_layout = InputLayout::new(&mut ctx)?;
    let input_funcs =
        InputFunctions::new(&mut ctx, &input_layout, state_funcs.pattern.lookup_start);
    let _matching_funcs = MatchingFunctions::new(
        &mut ctx,
        &state_layout,
        &state_funcs,
        &input_layout,
        &input_funcs,
    );
    let module: wasm_encoder::Module = ctx.compile(&state_layout.overall);

    Ok(RegexBytecode {
        bytes: module.finish().into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[track_caller]
    pub fn wasm_print_module(module_bytes: impl AsRef<[u8]>) -> String {
        let module_bytes = module_bytes.as_ref();
        let wasm_text = wasmprinter::print_bytes(module_bytes);
        if let Err(err) = wasmparser::validate(module_bytes) {
            let mut wasm_text_with_offsets = String::new();
            let print = wasmprinter::Config::new().print_offsets(true).print(
                module_bytes,
                &mut wasmprinter::PrintFmtWrite(&mut wasm_text_with_offsets),
            );

            match print {
                Ok(()) => {
                    panic!("{err}:\n{wasm_text_with_offsets}")
                },
                Err(print_err) => panic!("{err}:\nUnable to print WAT: {print_err}"),
            }
        }
        wasm_text.expect("should be able to print WASM module in WAT format")
    }

    /// A test helper function that compiles a regex pattern string into a
    /// [`CompiledRegex`].
    fn compile(pattern: &str) -> Result<RegexBytecode, Box<dyn std::error::Error>> {
        let nfa = regex_automata::nfa::thompson::NFA::new(pattern)?;

        Ok(compile_from_nfa(
            nfa,
            crate::Config::new().include_names(true),
        )?)
    }

    #[test]
    fn empty_regex() {
        let compiled = compile("").unwrap();
        let pretty = wasm_print_module(&compiled);
        insta::assert_snapshot!(pretty);
    }

    #[test]
    fn empty_pattern_list() {
        let nfa = regex_automata::nfa::thompson::NFA::new_many::<&str>(&[]).unwrap();
        let bytecode = compile_from_nfa(nfa, crate::Config::new().include_names(true)).unwrap();
        let pretty = wasm_print_module(&bytecode);
        insta::assert_snapshot!(pretty);
    }

    #[test]
    fn simple_repetition() {
        let compiled = compile("(?:abc)+").unwrap();
        let pretty = wasm_print_module(&compiled);
        insta::assert_snapshot!(pretty);
    }

    #[test]
    fn sparse_transitions() {
        let compiled = compile("a|b|d|e|g").unwrap();
        let pretty = wasm_print_module(&compiled);
        insta::assert_snapshot!(pretty);
    }

    #[test]
    fn simple_lookaround() {
        let compiled = compile("^hell worm$").unwrap();
        let pretty = wasm_print_module(&compiled);
        insta::assert_snapshot!(pretty);
    }

    #[test]
    fn repeated_lookaround() {
        let compiled = compile("(?:^|$)+").unwrap();
        let pretty = wasm_print_module(&compiled);
        insta::assert_snapshot!(pretty);
    }

    #[test]
    fn lookaround_crlf() {
        let compiled = compile("(?mR)^[a-z]+$").unwrap();
        let pretty = wasm_print_module(&compiled);
        insta::assert_snapshot!(pretty);
    }

    #[test]
    fn lookaround_lf() {
        let compiled = compile("(?m)^$").unwrap();
        let pretty = wasm_print_module(&compiled);
        insta::assert_snapshot!(pretty);
    }

    #[test]
    fn lookaround_is_word_ascii() {
        let compiled = compile(r"(?-u)hello\b").unwrap();
        let pretty = wasm_print_module(&compiled);
        insta::assert_snapshot!(pretty);
    }

    #[test]
    fn lookaround_is_word_ascii_negate() {
        let compiled = compile(r"(?-u)hello\B").unwrap();
        let pretty = wasm_print_module(&compiled);
        insta::assert_snapshot!(pretty);
    }

    #[test]
    fn lookaround_is_ascii_start_end() {
        let compiled = compile(r"(?-u:\b{start}hello\b{end})").unwrap();
        let pretty = wasm_print_module(&compiled);
        insta::assert_snapshot!(pretty);
    }

    #[test]
    fn lookaround_is_ascii_half_start_end() {
        let compiled = compile(r"(?-u:\b{start-half}hello\b{end-half})").unwrap();
        let pretty = wasm_print_module(&compiled);
        insta::assert_snapshot!(pretty);
    }

    #[test]
    fn lookaround_is_word_unicode() {
        let compiled = compile(r"(?u)hello\b").unwrap();
        let pretty = wasm_print_module(&compiled);
        insta::assert_snapshot!(pretty);
    }

    #[test]
    fn lookaround_is_word_unicode_negate() {
        let compiled = compile(r"(?u)hello\B").unwrap();
        let pretty = wasm_print_module(&compiled);
        insta::assert_snapshot!(pretty);
    }

    #[test]
    fn lookaround_is_unicode_start_end() {
        let compiled = compile(r"(?u:\b{start}hello\b{end})").unwrap();
        let pretty = wasm_print_module(&compiled);
        insta::assert_snapshot!(pretty);
    }

    #[test]
    fn lookaround_is_unicode_half_start_end() {
        let compiled = compile(r"(?u:\b{start-half}hello\b{end-half})").unwrap();
        let pretty = wasm_print_module(&compiled);
        insta::assert_snapshot!(pretty);
    }
}
