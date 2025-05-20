//! This module contains types and functions related to the implementation of
//! [`Look`]s in WASM.

use std::alloc::{Layout, LayoutError};

use regex_automata::util::look::Look;
use wasm_encoder::{NameMap, ValType};

use super::{
    context::{CompileContext, Function, FunctionDefinition, FunctionIdx, FunctionSignature},
    BuildError,
};

#[derive(Debug)]
pub struct LookLayout {}

impl LookLayout {
    /// TODO: Write docs for item
    pub fn new(_ctx: &mut CompileContext, overall: Layout) -> Result<(Layout, Self), LayoutError> {
        Ok((overall, Self {}))
    }
}

#[derive(Debug)]
pub struct LookFunctions {
    look_matches: [Option<FunctionIdx>; Self::NUM_LOOKS],
}

impl LookFunctions {
    const NUM_LOOKS: usize = const { (Look::WordEndHalfUnicode as usize).ilog2() as usize };

    /// TODO: Write docs for item
    pub fn new(ctx: &mut CompileContext, _layout: &LookLayout) -> Result<Self, BuildError> {
        let mut look_matches = [None; Self::NUM_LOOKS];

        for look in ctx.nfa.look_set_any().iter() {
            let func = match look {
                Look::Start => Self::is_start_fn(),
                Look::End => Self::is_end_fn(),
                Look::StartLF
                | Look::EndLF
                | Look::StartCRLF
                | Look::EndCRLF
                | Look::WordAscii
                | Look::WordAsciiNegate
                | Look::WordUnicode
                | Look::WordUnicodeNegate
                | Look::WordStartAscii
                | Look::WordEndAscii
                | Look::WordStartUnicode
                | Look::WordEndUnicode
                | Look::WordStartHalfAscii
                | Look::WordEndHalfAscii
                | Look::WordStartHalfUnicode
                | Look::WordEndHalfUnicode => {
                    return Err(BuildError::unsupported(format!(
                        "{look:?}/{} is not yet implemented",
                        look.as_char()
                    )))
                },
            };

            let func = ctx.add_function(func);
            look_matches[(look as usize).ilog2() as usize] = Some(func);
        }

        Ok(Self { look_matches })
    }

    /// TODO: Write docs for item
    pub fn look_matcher(&self, look: Look) -> Option<FunctionIdx> {
        self.look_matches[(look as usize).ilog2() as usize]
    }

    fn is_start_fn() -> Function {
        let mut locals_name_map = NameMap::new();
        // Parameters
        locals_name_map.append(0, "haystack_ptr");
        locals_name_map.append(1, "haystack_len");
        locals_name_map.append(2, "at_offset");

        // Sketch:
        // ```rust
        // return at_offset == 0;
        // ```

        let mut body = wasm_encoder::Function::new([]);
        // TODO(opt): Need to figure out a inlining strategy because this function is
        // tiny and wasmtime doesn't do inlining.
        body.instructions()
            // at_offset == 0
            .local_get(2)
            .i64_eqz()
            .end();

        Function {
            sig: FunctionSignature {
                name: "look_is_start".into(),
                // [haystack_ptr, haystack_len, at_offset]
                params_ty: &[ValType::I64, ValType::I64, ValType::I64],
                // [is_match]
                results_ty: &[ValType::I32],
                export: false,
            },
            def: FunctionDefinition {
                body,
                locals_name_map,
                labels_name_map: None,
                branch_hints: None,
            },
        }
    }

    fn is_end_fn() -> Function {
        let mut locals_name_map = NameMap::new();
        // Parameters
        locals_name_map.append(0, "haystack_ptr");
        locals_name_map.append(1, "haystack_len");
        locals_name_map.append(2, "at_offset");

        // Sketch:
        // ```rust
        // return at_offset == haystack_len;
        // ```

        let mut body = wasm_encoder::Function::new([]);
        body.instructions()
            // at_offset == haystack_len
            .local_get(1)
            .local_get(2)
            .i64_eq()
            .end();

        Function {
            sig: FunctionSignature {
                name: "look_is_end".into(),
                // [haystack_ptr, haystack_len, at_offset]
                params_ty: &[ValType::I64, ValType::I64, ValType::I64],
                // [is_match]
                results_ty: &[ValType::I32],
                export: false,
            },
            def: FunctionDefinition {
                body,
                locals_name_map,
                labels_name_map: None,
                branch_hints: None,
            },
        }
    }
}
