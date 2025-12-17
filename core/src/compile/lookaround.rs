//! This module contains types and functions related to the implementation of
//! [`Look`]s in WASM.

use std::alloc::{Layout, LayoutError};

use regex_automata::{
    nfa::thompson::NFA,
    util::look::{Look, LookMatcher, LookSet},
};
use wasm_encoder::{BlockType, InstructionSink, MemArg, NameMap, ValType};

use crate::compile::{
    context::FunctionTypeSignature,
    instructions::InstructionSinkExt,
    lookaround::{
        byte_word::IsWordByteLookupTable,
        perl_word_optimized::{PerlWordFunctions, PerlWordLayout},
    },
};

use super::context::{CompileContext, FunctionDefinition, FunctionIdx};

mod byte_word;
// code is generated, currently don't want to fix tool
#[expect(clippy::redundant_static_lifetimes)]
mod perl_word;
mod perl_word_optimized;

/// TODO: Write docs for this item
#[derive(Debug)]
pub struct LookLayout {
    is_word_byte_table: Option<IsWordByteLookupTable>,
    is_perl_word_table: Option<PerlWordLayout>,
}

impl LookLayout {
    /// TODO: Write docs for this item
    pub fn new(
        ctx: &mut CompileContext,
        mut overall: Layout,
    ) -> Result<(Layout, Self), LayoutError> {
        let look_set = modified_lookset_for_dependencies(&ctx.nfa);
        let is_word_byte_table = if needs_is_word_byte_lut(look_set) {
            let (new_overall, table) = IsWordByteLookupTable::new(ctx, overall)?;
            overall = new_overall;
            Some(table)
        } else {
            None
        };

        let is_perl_word_table = if needs_is_perl_word_lut(look_set) {
            let (new_overall, table) = PerlWordLayout::new(ctx, overall)?;
            overall = new_overall;
            Some(table)
        } else {
            None
        };

        Ok((
            overall,
            Self {
                is_word_byte_table,
                is_perl_word_table,
            },
        ))
    }
}

/// TODO: Write docs for this item
#[derive(Debug)]
pub struct LookFunctions {
    look_matches: [Option<FunctionIdx>; Self::NUM_LOOKS],
}

impl LookFunctions {
    // Look::WordEndHalfUnicode.as_repr() is 1 << 17. Its ilog2() is 17.
    // This means indices for look_matches range from 0 to 17, requiring an array of
    // size 18.
    const NUM_LOOKS: usize = (Look::WordEndHalfUnicode.as_repr().ilog2() as usize) + 1;

    /// TODO: Write docs for this item
    pub fn new(ctx: &mut CompileContext, layout: &LookLayout) -> Self {
        let mut look_matches = [None; Self::NUM_LOOKS];
        let look_set = modified_lookset_for_dependencies(&ctx.nfa);

        if look_set.is_empty() {
            return Self { look_matches };
        }

        let is_word_char_fns = if needs_is_perl_word_lut(look_set) {
            Some(PerlWordFunctions::new(
                ctx,
                layout
                    .is_perl_word_table
                    .as_ref()
                    .expect("perl layout should be present for functions"),
                layout
                    .is_word_byte_table
                    .as_ref()
                    .expect("word byte table should be present for perl word functions"),
            ))
        } else {
            None
        };

        let lookaround_fn_type = ctx.declare_fn_type(&FunctionTypeSignature {
            name: "lookaround",
            // [haystack_ptr, haystack_len, at_offset]
            params_ty: &[ValType::I64, ValType::I64, ValType::I64],
            // [is_match]
            results_ty: &[ValType::I32],
        });

        for look in look_set.iter() {
            let func =
                ctx.declare_function_with_type(lookaround_fn_type, lookaround_fn_name(look), false);
            look_matches[look.as_repr().ilog2() as usize] = Some(func);
        }

        let look_matcher = ctx.nfa.look_matcher().clone();
        for (look, func_idx) in look_set
            .iter()
            .map(|look| (look, look_matches[look.as_repr().ilog2() as usize]))
        {
            let func_def = match look {
                Look::Start => Self::is_start_fn(),
                Look::End => Self::is_end_fn(),
                Look::StartLF => Self::is_start_lf_fn(&look_matcher),
                Look::EndLF => Self::is_end_lf_fn(&look_matcher),
                Look::StartCRLF => Self::is_start_crlf_fn(),
                Look::EndCRLF => Self::is_end_crlf_fn(),
                Look::WordAscii => Self::is_word_ascii_fn(
                    layout
                        .is_word_byte_table
                        .as_ref()
                        .expect("should have generated table"),
                ),
                Look::WordAsciiNegate => Self::is_word_ascii_negate_fn(
                    look_matches[Look::WordAscii.as_repr().ilog2() as usize]
                        // See dependency in `modified_lookset_for_dependencies`
                        .expect("should have generated `look_is_word_ascii` function"),
                ),
                Look::WordStartAscii => Self::is_word_start_ascii_fn(
                    layout
                        .is_word_byte_table
                        .as_ref()
                        .expect("should have generated table"),
                ),
                Look::WordEndAscii => Self::is_word_end_ascii_fn(
                    layout
                        .is_word_byte_table
                        .as_ref()
                        .expect("should have generated table"),
                ),
                Look::WordStartHalfAscii => Self::is_word_start_half_ascii_fn(
                    layout
                        .is_word_byte_table
                        .as_ref()
                        .expect("should have generated table"),
                ),
                Look::WordEndHalfAscii => Self::is_word_end_half_ascii_fn(
                    layout
                        .is_word_byte_table
                        .as_ref()
                        .expect("should have generated table"),
                ),
                Look::WordUnicode => {
                    let perl_word_fns = is_word_char_fns
                        .as_ref()
                        .expect("should have generated helper functions");
                    Self::is_word_unicode_fn(
                        perl_word_fns.is_word_char_rev,
                        perl_word_fns.is_word_char_fwd,
                    )
                },
                Look::WordUnicodeNegate => {
                    let perl_word_fns = is_word_char_fns
                        .as_ref()
                        .expect("should have generated helper functions");
                    Self::is_word_unicode_negate_fn(
                        perl_word_fns.is_word_character,
                        perl_word_fns.decode_last_character,
                        perl_word_fns.decode_next_character,
                    )
                },
                Look::WordStartUnicode => {
                    let perl_word_fns = is_word_char_fns
                        .as_ref()
                        .expect("should have generated helper functions");
                    Self::is_word_start_unicode_fn(
                        perl_word_fns.is_word_char_rev,
                        perl_word_fns.is_word_char_fwd,
                    )
                },
                Look::WordEndUnicode => {
                    let perl_word_fns = is_word_char_fns
                        .as_ref()
                        .expect("should have generated helper functions");
                    Self::is_word_end_unicode_fn(
                        perl_word_fns.is_word_char_rev,
                        perl_word_fns.is_word_char_fwd,
                    )
                },
                Look::WordStartHalfUnicode => {
                    let perl_word_fns = is_word_char_fns
                        .as_ref()
                        .expect("should have generated helper functions");
                    Self::is_word_start_half_unicode_fn(
                        perl_word_fns.is_word_character,
                        perl_word_fns.decode_last_character,
                    )
                },
                Look::WordEndHalfUnicode => {
                    let perl_word_fns = is_word_char_fns
                        .as_ref()
                        .expect("should have generated helper functions");
                    Self::is_word_end_half_unicode_fn(
                        perl_word_fns.is_word_character,
                        perl_word_fns.decode_next_character,
                    )
                },
            };

            ctx.define_function(
                func_idx.expect("should have generated function for supported lookaround"),
                func_def,
            );
        }

        Self { look_matches }
    }

    /// TODO: Write docs for this item
    pub fn look_matcher(&self, look: Look) -> Option<FunctionIdx> {
        self.look_matches[look.as_repr().ilog2() as usize]
    }

    fn is_start_fn() -> FunctionDefinition {
        let locals_name_map = lookaround_fn_common_name_map();

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

        FunctionDefinition {
            body,
            locals_name_map,
            labels_name_map: None,
            branch_hints: None,
        }
    }

    fn is_end_fn() -> FunctionDefinition {
        let locals_name_map = lookaround_fn_common_name_map();

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

        FunctionDefinition {
            body,
            locals_name_map,
            labels_name_map: None,
            branch_hints: None,
        }
    }

    fn is_start_lf_fn(look_matcher: &LookMatcher) -> FunctionDefinition {
        let locals_name_map = lookaround_fn_common_name_map();

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
            .bool_const(true)
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

        FunctionDefinition {
            body,
            locals_name_map,
            labels_name_map: None,
            branch_hints: None,
        }
    }

    fn is_end_lf_fn(look_matcher: &LookMatcher) -> FunctionDefinition {
        let locals_name_map = lookaround_fn_common_name_map();

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
            .bool_const(true)
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

        FunctionDefinition {
            body,
            locals_name_map,
            labels_name_map: None,
            branch_hints: None,
        }
    }

    fn is_start_crlf_fn() -> FunctionDefinition {
        let locals_name_map = lookaround_fn_common_name_map();

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
            .bool_const(true)
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
            .bool_const(true)
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
            .bool_const(false)
            .return_()
            .end()
            // at >= haystack_len
            .local_get(2)
            .local_get(1)
            .i64_ge_u()
            .if_(BlockType::Empty)
            .bool_const(true)
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

        FunctionDefinition {
            body,
            locals_name_map,
            labels_name_map: None,
            branch_hints: None,
        }
    }

    fn is_end_crlf_fn() -> FunctionDefinition {
        let locals_name_map = lookaround_fn_common_name_map();

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
            .bool_const(true)
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
            .bool_const(true)
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
            .bool_const(false)
            .return_()
            .end()
            // at == 0
            .local_get(2)
            .i64_eqz()
            .if_(BlockType::Empty)
            .bool_const(true)
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

        FunctionDefinition {
            body,
            locals_name_map,
            labels_name_map: None,
            branch_hints: None,
        }
    }

    fn is_word_ascii_fn(is_word_byte_table: &IsWordByteLookupTable) -> FunctionDefinition {
        let mut locals_name_map = lookaround_fn_common_name_map();
        // Locals
        locals_name_map.append(3, "word_before");

        // Sketch:
        // ```rust
        // ...
        // return word_before != word_after;
        // ```

        let mut body = wasm_encoder::Function::new([(2, ValType::I32)]);

        let mut instructions = body.instructions();
        Self::word_before_ascii_instructions(&mut instructions, is_word_byte_table);
        instructions.local_set(3);

        Self::word_after_ascii_instructions(&mut instructions, is_word_byte_table);
        instructions
            // return word_before != word_after;
            .local_get(3)
            .i32_ne()
            .end();

        FunctionDefinition {
            body,
            locals_name_map,
            labels_name_map: None,
            branch_hints: None,
        }
    }

    fn is_word_ascii_negate_fn(is_word_ascii: FunctionIdx) -> FunctionDefinition {
        let locals_name_map = lookaround_fn_common_name_map();

        // Sketch:
        // ```rust
        // return !look_is_word_ascii(haystack_ptr, haystack_len, at_offset);
        // ```

        let mut body = wasm_encoder::Function::new([]);
        body.instructions()
            .local_get(0)
            .local_get(1)
            .local_get(2)
            .call(is_word_ascii.into())
            .i32_const(1)
            .i32_xor()
            .end();

        FunctionDefinition {
            body,
            locals_name_map,
            labels_name_map: None,
            branch_hints: None,
        }
    }

    fn is_word_start_ascii_fn(is_word_byte_table: &IsWordByteLookupTable) -> FunctionDefinition {
        let mut locals_name_map = lookaround_fn_common_name_map();
        // Locals
        locals_name_map.append(3, "word_before");

        // Sketch:
        // ```rust
        // ...
        // return !word_before && word_after;
        // ```

        let mut body = wasm_encoder::Function::new([(2, ValType::I32)]);

        let mut instructions = body.instructions();
        Self::word_before_ascii_instructions(&mut instructions, is_word_byte_table);
        instructions.local_set(3);

        Self::word_after_ascii_instructions(&mut instructions, is_word_byte_table);
        instructions
            // return !word_before && word_after;
            .local_get(3)
            .i32_const(1)
            .i32_xor()
            .i32_and()
            .end();

        FunctionDefinition {
            body,
            locals_name_map,
            labels_name_map: None,
            branch_hints: None,
        }
    }

    fn is_word_end_ascii_fn(is_word_byte_table: &IsWordByteLookupTable) -> FunctionDefinition {
        let mut locals_name_map = lookaround_fn_common_name_map();
        // Locals
        locals_name_map.append(3, "word_before");

        // Sketch:
        // ```rust
        // ...
        // return word_before && !word_after;
        // ```

        let mut body = wasm_encoder::Function::new([(2, ValType::I32)]);

        let mut instructions = body.instructions();
        Self::word_before_ascii_instructions(&mut instructions, is_word_byte_table);
        instructions.local_set(3);

        Self::word_after_ascii_instructions(&mut instructions, is_word_byte_table);
        instructions
            // return word_before && !word_after;
            .i32_const(1)
            .i32_xor()
            .local_get(3)
            .i32_and()
            .end();

        FunctionDefinition {
            body,
            locals_name_map,
            labels_name_map: None,
            branch_hints: None,
        }
    }

    fn is_word_start_half_ascii_fn(
        is_word_byte_table: &IsWordByteLookupTable,
    ) -> FunctionDefinition {
        let locals_name_map = lookaround_fn_common_name_map();

        // Sketch:
        // ```rust
        // ...
        // return !word_before;
        // ```

        let mut body = wasm_encoder::Function::new([]);

        let mut instructions = body.instructions();
        Self::word_before_ascii_instructions(&mut instructions, is_word_byte_table);
        instructions
            // return !word_before;
            .i32_const(1)
            .i32_xor()
            .end();

        FunctionDefinition {
            body,
            locals_name_map,
            labels_name_map: None,
            branch_hints: None,
        }
    }

    fn is_word_end_half_ascii_fn(is_word_byte_table: &IsWordByteLookupTable) -> FunctionDefinition {
        let locals_name_map = lookaround_fn_common_name_map();

        // Sketch:
        // ```rust
        // ...
        // return !word_after;
        // ```

        let mut body = wasm_encoder::Function::new([]);
        let mut instructions = body.instructions();

        Self::word_after_ascii_instructions(&mut instructions, is_word_byte_table);
        instructions
            // return !word_after;
            .i32_const(1)
            .i32_xor()
            .end();

        FunctionDefinition {
            body,
            locals_name_map,
            labels_name_map: None,
            branch_hints: None,
        }
    }

    fn is_word_unicode_fn(
        is_word_char_rev: FunctionIdx,
        is_word_char_fwd: FunctionIdx,
    ) -> FunctionDefinition {
        let locals_name_map = lookaround_fn_common_name_map();

        // Sketch:
        // ```rust
        // let word_before = is_word_char::rev(haystack, at);
        // let word_after = is_word_char::fwd(haystack, at);
        // return word_before != word_after;
        // ```

        let mut body = wasm_encoder::Function::new([]);
        let mut instructions = body.instructions();

        instructions
            .local_get(0)
            .local_get(1)
            .local_get(2)
            // let word_before = is_word_char::rev(haystack, at)?;
            .call(is_word_char_rev.into())
            .local_get(0)
            .local_get(1)
            .local_get(2)
            // let word_after = is_word_char::fwd(haystack, at)?;
            .call(is_word_char_fwd.into())
            // return word_before != word_after;
            // a | b | a != b | a XOR B
            // T | T | F | F
            // T | F | T | T
            // F | T | T | T
            // F | F | F | F
            .i32_xor()
            .end();

        FunctionDefinition {
            body,
            locals_name_map,
            labels_name_map: None,
            branch_hints: None,
        }
    }

    fn is_word_unicode_negate_fn(
        is_word_character: FunctionIdx,
        decode_last_character: FunctionIdx,
        decode_next_character: FunctionIdx,
    ) -> FunctionDefinition {
        let mut locals_name_map = lookaround_fn_common_name_map();
        // Locals
        locals_name_map.append(3, "word_before");
        locals_name_map.append(4, "word_after");
        locals_name_map.append(5, "character");
        let mut labels_name_map = NameMap::new();
        labels_name_map.append(0, "check_before_start");
        labels_name_map.append(1, "check_last_invalid_char");
        labels_name_map.append(2, "check_after_start");
        labels_name_map.append(3, "check_next_invalid_char");

        // Sketch:
        // ```rust
        // word_before = false
        // if at_offset > 0 {
        //     let (character, _) = utf8_decode_last_character(haystack_ptr, at_offset)
        //     if character == INVALID_CHAR {
        //         return false
        //     } else {
        //         word_before = is_word_character(character)
        //     }
        // }
        //
        // word_after = false
        // if at_offset < haystack_len {
        //     let (character, _) = utf8_decode_next_character(haystack_ptr + at_offset, haystack_len - at_offset)
        //     if character == INVALID_CHAR {
        //         return false
        //     } else {
        //         word_after = is_word_character(character)
        //     }
        // }
        //
        // return word_before == word_after
        // ```

        let mut body = wasm_encoder::Function::new([(3, ValType::I32)]);
        let mut instructions = body.instructions();

        instructions
            // if at_offset > 0 {
            .local_get(2)
            .i64_const(0)
            .i64_gt_u()
            .if_(BlockType::Empty)
            //     let (character, _) = utf8_decode_last_character(haystack_ptr, at_offset)
            .local_get(0)
            .local_get(2)
            .call(decode_last_character.into())
            .drop()
            .local_tee(5)
            //     if character == INVALID_CHAR {
            .u32_const(PerlWordFunctions::INVALID_CHAR)
            .i32_eq()
            .if_(BlockType::Empty)
            //         return false
            .bool_const(false)
            .return_()
            //     } else {
            .else_()
            //         word_after = is_word_character(character)
            .local_get(5)
            .call(is_word_character.into())
            .local_set(3)
            //     } - end inner if
            .end()
            // } - end outer if
            .end()
            // if at_offset < haystack_len {
            .local_get(2)
            .local_get(1)
            .i64_lt_u()
            .if_(BlockType::Empty)
            //     let haystack_slice_ptr = haystack_ptr + at_offset
            //     let haystack_slice_len = haystack_len - at_offset
            //     let (character, _) = utf8_decode_next_character(.., ..)
            .local_get(0)
            .local_get(2)
            .i64_add()
            .local_get(1)
            .local_get(2)
            .i64_sub()
            .call(decode_next_character.into())
            .drop()
            .local_tee(5)
            //     if character == INVALID_CHAR {
            .u32_const(PerlWordFunctions::INVALID_CHAR)
            .i32_eq()
            .if_(BlockType::Empty)
            //         return false
            .bool_const(false)
            .return_()
            //     } else {
            .else_()
            //         word_after = is_word_character(character)
            .local_get(5)
            .call(is_word_character.into())
            .local_set(4)
            //     } - end inner if
            .end()
            // } - end outer if
            .end()
            // return word_before == word_after
            .local_get(3)
            .local_get(4)
            .i32_eq()
            .end();

        FunctionDefinition {
            body,
            locals_name_map,
            labels_name_map: None,
            branch_hints: None,
        }
    }

    fn is_word_start_unicode_fn(
        is_word_char_rev: FunctionIdx,
        is_word_char_fwd: FunctionIdx,
    ) -> FunctionDefinition {
        let locals_name_map = lookaround_fn_common_name_map();

        // Sketch:
        // ```rust
        // let word_before = is_word_char::rev(haystack, at);
        // let word_after = is_word_char::fwd(haystack, at);
        // return !word_before && word_after
        // ```

        let mut body = wasm_encoder::Function::new([]);
        let mut instructions = body.instructions();

        instructions
            .local_get(0)
            .local_get(1)
            .local_get(2)
            // let word_before = is_word_char::rev(haystack, at)?;
            .call(is_word_char_rev.into())
            // !word_before
            .u32_const(1)
            .i32_xor()
            .local_get(0)
            .local_get(1)
            .local_get(2)
            // let word_after = is_word_char::fwd(haystack, at)?;
            .call(is_word_char_fwd.into())
            // return !word_before && word_after
            .i32_and()
            .end();

        FunctionDefinition {
            body,
            locals_name_map,
            labels_name_map: None,
            branch_hints: None,
        }
    }

    fn is_word_end_unicode_fn(
        is_word_char_rev: FunctionIdx,
        is_word_char_fwd: FunctionIdx,
    ) -> FunctionDefinition {
        let locals_name_map = lookaround_fn_common_name_map();

        // Sketch:
        // ```rust
        // let word_before = is_word_char::rev(haystack, at);
        // let word_after = is_word_char::fwd(haystack, at);
        // return word_before && !word_after
        // ```

        let mut body = wasm_encoder::Function::new([]);
        let mut instructions = body.instructions();

        instructions
            .local_get(0)
            .local_get(1)
            .local_get(2)
            // let word_before = is_word_char::rev(haystack, at)?;
            .call(is_word_char_rev.into())
            .local_get(0)
            .local_get(1)
            .local_get(2)
            // let word_after = is_word_char::fwd(haystack, at)?;
            .call(is_word_char_fwd.into())
            .u32_const(1)
            .i32_xor()
            // return word_before && !word_after
            .i32_and()
            .end();

        FunctionDefinition {
            body,
            locals_name_map,
            labels_name_map: None,
            branch_hints: None,
        }
    }

    fn is_word_start_half_unicode_fn(
        is_word_character: FunctionIdx,
        decode_last_character: FunctionIdx,
    ) -> FunctionDefinition {
        let mut locals_name_map = lookaround_fn_common_name_map();
        // Locals
        locals_name_map.append(3, "word_before");
        locals_name_map.append(4, "character");
        let mut labels_name_map = NameMap::new();
        labels_name_map.append(0, "check_before_start");
        labels_name_map.append(1, "check_last_invalid_char");

        // Sketch:
        // ```rust
        // word_before = false
        // if at_offset > 0 {
        //     let (character, _) = utf8_decode_last_character(haystack_ptr, at_offset)
        //     if character == INVALID_CHAR {
        //         return false
        //     } else {
        //         word_before = is_word_character(character)
        //     }
        // }
        //
        // return !word_before
        // ```

        let mut body = wasm_encoder::Function::new([(2, ValType::I32)]);
        let mut instructions = body.instructions();

        instructions
            // if at_offset > 0 {
            .local_get(2)
            .i64_const(0)
            .i64_gt_u()
            .if_(BlockType::Empty)
            //     let (character, _) = utf8_decode_last_character(haystack_ptr, at_offset)
            .local_get(0)
            .local_get(2)
            .call(decode_last_character.into())
            .drop()
            .local_tee(4)
            //     if character == INVALID_CHAR {
            .u32_const(PerlWordFunctions::INVALID_CHAR)
            .i32_eq()
            .if_(BlockType::Empty)
            //         return false
            .bool_const(false)
            .return_()
            //     } else {
            .else_()
            //         word_after = is_word_character(character)
            .local_get(4)
            .call(is_word_character.into())
            .local_set(3)
            //     } - end inner if
            .end()
            // } - end outer if
            .end()
            // return !word_before
            .local_get(3)
            .u32_const(1)
            .i32_xor()
            .end();

        FunctionDefinition {
            body,
            locals_name_map,
            labels_name_map: None,
            branch_hints: None,
        }
    }

    fn is_word_end_half_unicode_fn(
        is_word_character: FunctionIdx,
        decode_next_character: FunctionIdx,
    ) -> FunctionDefinition {
        let mut locals_name_map = lookaround_fn_common_name_map();
        // Locals
        locals_name_map.append(3, "word_after");
        locals_name_map.append(4, "character");
        let mut labels_name_map = NameMap::new();
        labels_name_map.append(0, "check_after_start");
        labels_name_map.append(1, "check_next_invalid_char");

        // Sketch:
        // ```rust
        // word_after = false
        // if at_offset < haystack_len {
        //     let (character, _) = utf8_decode_next_character(haystack_ptr + at_offset, haystack_len - at_offset)
        //     if character == INVALID_CHAR {
        //         return false
        //     } else {
        //         word_after = is_word_character(character)
        //     }
        // }
        //
        // return !word_after
        // ```

        let mut body = wasm_encoder::Function::new([(2, ValType::I32)]);
        let mut instructions = body.instructions();

        instructions
            // if at_offset < haystack_len {
            .local_get(2)
            .local_get(1)
            .i64_lt_u()
            .if_(BlockType::Empty)
            //     let haystack_slice_ptr = haystack_ptr + at_offset
            //     let haystack_slice_len = haystack_len - at_offset
            //     let (character, _) = utf8_decode_next_character(.., ..)
            .local_get(0)
            .local_get(2)
            .i64_add()
            .local_get(1)
            .local_get(2)
            .i64_sub()
            .call(decode_next_character.into())
            .drop()
            .local_tee(4)
            //     if character == INVALID_CHAR {
            .u32_const(PerlWordFunctions::INVALID_CHAR)
            .i32_eq()
            .if_(BlockType::Empty)
            //         return false
            .bool_const(false)
            .return_()
            //     } else {
            .else_()
            //         word_after = is_word_character(character)
            .local_get(4)
            .call(is_word_character.into())
            .local_set(3)
            //     } - end inner if
            .end()
            // } - end outer if
            .end()
            // return !word_after
            .local_get(3)
            .u32_const(1)
            .i32_xor()
            .end();

        FunctionDefinition {
            body,
            locals_name_map,
            labels_name_map: None,
            branch_hints: None,
        }
    }

    fn word_before_ascii_instructions(
        instructions: &mut InstructionSink,
        is_word_byte_table: &IsWordByteLookupTable,
    ) {
        // Sketch:
        // ```rust
        // if at_offset == 0 {
        //    false
        // } else {
        //    is_word_byte_table[haystack_ptr[at_offset - 1]]
        // }
        // ```

        instructions
            // if at_offset == 0 {
            .local_get(2)
            .i64_eqz()
            .if_(BlockType::Result(ValType::I32))
            .bool_const(false)
            .else_()
            // word_before = is_word_byte_table[haystack_ptr[at_offset - 1]];
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
            .i64_extend_i32_u()
            .i32_load8_u(MemArg {
                offset: is_word_byte_table.position(),
                align: 0, // byte alignment
                memory_index: 1,
            })
            .end();
    }

    fn word_after_ascii_instructions(
        instructions: &mut InstructionSink,
        is_word_byte_table: &IsWordByteLookupTable,
    ) {
        // Sketch:
        // ```rust
        // if at_offset >= haystack_len {
        //    false
        // } else {
        //    is_word_byte_table[haystack_ptr[at_offset]]
        // }
        // ```

        instructions
            // let word_after;
            // if at_offset >= haystack_len {
            .local_get(2)
            .local_get(1)
            .i64_ge_u()
            .if_(BlockType::Result(ValType::I32))
            .bool_const(false)
            .else_()
            // word_after = is_word_byte_table[haystack_ptr[at_offset]];
            .local_get(2)
            .local_get(0)
            .i64_add()
            .i32_load8_u(MemArg {
                offset: 0,
                align: 0,        // byte alignment
                memory_index: 0, // haystack
            })
            .i64_extend_i32_u()
            .i32_load8_u(MemArg {
                offset: is_word_byte_table.position(),
                align: 0, // byte alignment
                memory_index: 1,
            })
            .end();
    }
}

fn lookaround_fn_common_name_map() -> NameMap {
    let mut locals_name_map = NameMap::new();
    locals_name_map.append(0, "haystack_ptr");
    locals_name_map.append(1, "haystack_len");
    locals_name_map.append(2, "at_offset");

    locals_name_map
}

fn lookaround_fn_name(look: Look) -> &'static str {
    match look {
        Look::Start => "look_is_start",
        Look::End => "look_is_end",
        Look::StartLF => "look_is_start_lf",
        Look::EndLF => "look_is_end_lf",
        Look::StartCRLF => "look_is_start_crlf",
        Look::EndCRLF => "look_is_end_crlf",
        Look::WordAscii => "look_is_word_ascii",
        Look::WordAsciiNegate => "look_is_word_ascii_negate",
        Look::WordUnicode => "look_is_word_unicode",
        Look::WordUnicodeNegate => "look_is_word_unicode_negate",
        Look::WordStartAscii => "look_is_word_start_ascii",
        Look::WordEndAscii => "look_is_word_end_ascii",
        Look::WordStartUnicode => "look_is_word_start_unicode",
        Look::WordEndUnicode => "look_is_word_end_unicode",
        Look::WordStartHalfAscii => "look_is_word_start_half_ascii",
        Look::WordEndHalfAscii => "look_is_word_end_half_ascii",
        Look::WordStartHalfUnicode => "look_is_word_start_half_unicode",
        Look::WordEndHalfUnicode => "look_is_word_end_half_unicode",
    }
}

fn needs_is_word_byte_lut(look_set: LookSet) -> bool {
    look_set.contains_word_ascii() || needs_is_perl_word_lut(look_set)
}

fn needs_is_perl_word_lut(look_set: LookSet) -> bool {
    look_set.contains_word_unicode()
}

fn modified_lookset_for_dependencies(nfa: &NFA) -> LookSet {
    let mut look_set = nfa.look_set_any();

    // This dependency exists because `look_is_word_ascii_negate` directly calls
    // `look_is_word_ascii`
    if look_set.contains(Look::WordAsciiNegate) {
        look_set = look_set.insert(Look::WordAscii);
    }

    look_set
}
