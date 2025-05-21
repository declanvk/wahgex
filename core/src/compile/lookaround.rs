//! This module contains types and functions related to the implementation of
//! [`Look`]s in WASM.

use std::alloc::{Layout, LayoutError};

use regex_automata::util::look::{Look, LookMatcher};
use wasm_encoder::{BlockType, MemArg, NameMap, ValType};

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
        let look_matcher = ctx.nfa.look_matcher().clone();
        let mut look_matches = [None; Self::NUM_LOOKS];

        for look in ctx.nfa.look_set_any().iter() {
            let func = match look {
                Look::Start => Self::is_start_fn(),
                Look::End => Self::is_end_fn(),
                Look::StartLF => Self::is_start_lf_fn(&look_matcher),
                Look::EndLF => Self::is_end_lf_fn(&look_matcher),
                Look::StartCRLF => Self::is_start_crlf_fn(),
                Look::EndCRLF => Self::is_end_crlf_fn(),
                Look::WordAscii
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
                    // TODO: Need to implement the rest of the lookaround assertions
                    return Err(BuildError::unsupported(format!(
                        "{look:?}/{} is not yet implemented",
                        look.as_char()
                    )));
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
            sig: Self::lookaround_fn_signature("look_is_start"),
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
            sig: Self::lookaround_fn_signature("look_is_end"),
            def: FunctionDefinition {
                body,
                locals_name_map,
                labels_name_map: None,
                branch_hints: None,
            },
        }
    }

    fn is_start_lf_fn(look_matcher: &LookMatcher) -> Function {
        let mut locals_name_map = NameMap::new();
        // Parameters
        locals_name_map.append(0, "haystack_ptr");
        locals_name_map.append(1, "haystack_len");
        locals_name_map.append(2, "at_offset");

        // Sketch:
        // ```rust
        // return at_offset == 0 || haystack[at_offset - 1] == lineterm
        // ```

        let mut body = wasm_encoder::Function::new([]);
        body.instructions()
            // at_offset == 0
            .local_get(2)
            .i64_eqz()
            .if_(BlockType::Empty)
            // TODO(opt): is the branch better here? Or should it just be an unconditional i32.or
            .i32_const(true as i32)
            .return_()
            .end()
            // haystack[at_offset - 1] == lineterm
            .local_get(2)
            .i64_const(1)
            .i64_sub()
            .local_get(0)
            .i64_add()
            .i32_load8_u(MemArg {
                offset: 0,
                align: 0,        // loading single byte
                memory_index: 0, // haystack
            })
            .i32_const(look_matcher.get_line_terminator() as i32)
            .i32_eq()
            .end();

        Function {
            sig: Self::lookaround_fn_signature("look_is_start_lf"),
            def: FunctionDefinition {
                body,
                locals_name_map,
                labels_name_map: None,
                branch_hints: None,
            },
        }
    }

    fn is_end_lf_fn(look_matcher: &LookMatcher) -> Function {
        let mut locals_name_map = NameMap::new();
        // Parameters
        locals_name_map.append(0, "haystack_ptr");
        locals_name_map.append(1, "haystack_len");
        locals_name_map.append(2, "at_offset");

        // Sketch:
        // ```rust
        // return at_offset == haystack_len || haystack[at] == self.lineterm.0
        // ```

        let mut body = wasm_encoder::Function::new([]);
        body.instructions()
            // at_offset == haystack_len
            .local_get(2)
            .local_get(1)
            .i64_eq()
            .if_(BlockType::Empty)
            // TODO(opt): is the branch better here? Or should it just be an unconditional i32.or
            .i32_const(true as i32)
            .return_()
            .end()
            // haystack[at_offset] == lineterm
            .local_get(2)
            .local_get(0)
            .i64_add()
            .i32_load8_u(MemArg {
                offset: 0,
                align: 0,        // loading single byte
                memory_index: 0, // haystack
            })
            .i32_const(look_matcher.get_line_terminator() as i32)
            .i32_eq()
            .end();

        Function {
            sig: Self::lookaround_fn_signature("look_is_end_lf"),
            def: FunctionDefinition {
                body,
                locals_name_map,
                labels_name_map: None,
                branch_hints: None,
            },
        }
    }

    fn is_start_crlf_fn() -> Function {
        let mut locals_name_map = NameMap::new();
        // Parameters
        locals_name_map.append(0, "haystack_ptr");
        locals_name_map.append(1, "haystack_len");
        locals_name_map.append(2, "at_offset");

        // Sketch:
        // ```rust
        // if at_offset == 0 {
        //     return true;
        // }
        // if haystack[at_offset - 1] == b'\n' {
        //     return true;
        // }
        // if haystack[at_offset - 1] != b'\r' {
        //     return false;
        // }
        // if at_offset >= haystack_len {
        //     return true;
        // }
        // return haystack[at_offset] != b'\n';
        // ```

        let mut body = wasm_encoder::Function::new([]);
        body.instructions()
            // at == 0
            .local_get(2)
            .i64_eqz()
            .if_(BlockType::Empty)
            .i32_const(true as i32)
            .return_()
            .end()
            // haystack[at - 1] == b'\n'
            .local_get(2)
            .i64_const(1)
            .i64_sub()
            .local_get(0)
            .i64_add()
            .i32_load8_u(MemArg {
                offset: 0,
                align: 0,        // byte alignment
                memory_index: 0, // haystack
            })
            .i32_const(b'\n' as i32)
            .i32_eq()
            .if_(BlockType::Empty)
            .i32_const(true as i32)
            .return_()
            .end()
            // haystack[at - 1] != b'\r'
            .local_get(2)
            .i64_const(1)
            .i64_sub()
            .local_get(0)
            .i64_add()
            .i32_load8_u(MemArg {
                offset: 0,
                align: 0,        // byte alignment
                memory_index: 0, // haystack
            })
            .i32_const(b'\r' as i32)
            .i32_ne()
            .if_(BlockType::Empty)
            .i32_const(false as i32)
            .return_()
            .end()
            // at >= haystack_len
            .local_get(2)
            .local_get(1)
            .i64_ge_u()
            .if_(BlockType::Empty)
            .i32_const(true as i32)
            .return_()
            .end()
            // haystack[at] != b'\n'
            .local_get(2)
            .local_get(0)
            .i64_add()
            .i32_load8_u(MemArg {
                offset: 0,
                align: 0,        // byte alignment
                memory_index: 0, // haystack
            })
            .i32_const(b'\n' as i32)
            .i32_ne()
            .end();

        Function {
            sig: Self::lookaround_fn_signature("look_is_start_crlf"),
            def: FunctionDefinition {
                body,
                locals_name_map,
                labels_name_map: None,
                branch_hints: None,
            },
        }
    }

    fn is_end_crlf_fn() -> Function {
        let mut locals_name_map = NameMap::new();
        // Parameters
        locals_name_map.append(0, "haystack_ptr");
        locals_name_map.append(1, "haystack_len");
        locals_name_map.append(2, "at_offset");

        // Sketch:
        // ```rust
        // at == haystack.len()
        //     || haystack[at] == b'\r'
        //     || (haystack[at] == b'\n'
        //         && (at == 0 || haystack[at - 1] != b'\r'))
        // if at == haystack.len() {
        //     return true;
        // }
        // if haystack[at] == b'\r' {
        //     return true;
        // }
        // if haystack[at] != b'\n' {
        //     return false;
        // }
        // if at == 0 {
        //     return true;
        // }
        // return haystack[at - 1] != b'\r';
        // ```

        let mut body = wasm_encoder::Function::new([]);

        body.instructions()
            // at == haystack.len()
            .local_get(2)
            .local_get(1)
            .i64_eq()
            .if_(BlockType::Empty)
            .i32_const(true as i32)
            .return_()
            .end()
            // haystack[at] == b'\r'
            .local_get(2)
            .local_get(0)
            .i64_add()
            .i32_load8_u(MemArg {
                offset: 0,
                align: 0,        // byte alignment
                memory_index: 0, // haystack
            })
            .i32_const(b'\r' as i32)
            .i32_eq()
            .if_(BlockType::Empty)
            .i32_const(true as i32)
            .return_()
            .end()
            // haystack[at] != b'\n'
            .local_get(2)
            .local_get(0)
            .i64_add()
            .i32_load8_u(MemArg {
                offset: 0,
                align: 0,        // byte alignment
                memory_index: 0, // haystack
            })
            .i32_const(b'\n' as i32)
            .i32_ne()
            .if_(BlockType::Empty)
            .i32_const(false as i32)
            .return_()
            .end()
            // at == 0
            .local_get(2)
            .i64_eqz()
            .if_(BlockType::Empty)
            .i32_const(true as i32)
            .return_()
            .end()
            // haystack[at - 1] != b'\r'
            .local_get(2)
            .i64_const(1)
            .i64_sub()
            .local_get(0)
            .i64_add()
            .i32_load8_u(MemArg {
                offset: 0,
                align: 0,        // byte alignment
                memory_index: 0, // haystack
            })
            .i32_const(b'\r' as i32)
            .i32_ne()
            .end();

        Function {
            sig: Self::lookaround_fn_signature("look_is_end_crlf"),
            def: FunctionDefinition {
                body,
                locals_name_map,
                labels_name_map: None,
                branch_hints: None,
            },
        }
    }

    fn lookaround_fn_signature(name: &str) -> FunctionSignature {
        FunctionSignature {
            name: name.into(),
            // [haystack_ptr, haystack_len, at_offset]
            params_ty: &[ValType::I64, ValType::I64, ValType::I64],
            // [is_match]
            results_ty: &[ValType::I32],
            export: false,
        }
    }
}
