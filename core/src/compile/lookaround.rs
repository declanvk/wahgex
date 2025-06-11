//! This module contains types and functions related to the implementation of
//! [`Look`]s in WASM.

use std::alloc::{Layout, LayoutError};

use regex_automata::{
    nfa::thompson::NFA,
    util::look::{Look, LookMatcher, LookSet},
};
use wasm_encoder::{BlockType, InstructionSink, MemArg, NameMap, ValType};

use crate::util::repeat;

use super::{
    context::{
        ActiveDataSegment, CompileContext, FunctionDefinition, FunctionIdx, FunctionSignature,
    },
    BuildError,
};

/// TODO: Write docs for this item
#[derive(Debug)]
pub struct LookLayout {
    is_word_byte_table: Option<IsWordByteTable>,
}

#[derive(Debug)]
struct IsWordByteTable {
    position: u64,
}

impl LookLayout {
    /// This lookup table is true for bytes which are considered "word" unicode
    /// characters.
    ///
    /// The logic is copied directly from <https://github.com/rust-lang/regex/blob/master/regex-automata/src/util/utf8.rs#L17-L37>
    /// As the comment on the function (in the link) mentions, no bit-rot
    /// because this will not change.
    const UTF8_IS_WORD_BYTE_LUT: [bool; 256] = {
        let mut set = [false; 256];
        set[b'_' as usize] = true;

        let mut byte = b'0';
        while byte <= b'9' {
            set[byte as usize] = true;
            byte += 1;
        }
        byte = b'A';
        while byte <= b'Z' {
            set[byte as usize] = true;
            byte += 1;
        }
        byte = b'a';
        while byte <= b'z' {
            set[byte as usize] = true;
            byte += 1;
        }
        set
    };

    /// TODO: Write docs for item
    pub fn new(
        ctx: &mut CompileContext,
        mut overall: Layout,
    ) -> Result<(Layout, Self), LayoutError> {
        let look_set = modified_lookset_for_dependencies(&ctx.nfa);
        let is_word_byte_table = if needs_is_word_byte_lut(look_set) {
            let (table_layout, _table_stride) = repeat(&Layout::new::<u8>(), 256)?;

            let (new_overall, table_pos) = overall.extend(table_layout)?;
            overall = new_overall;

            ctx.sections.add_active_data_segment(ActiveDataSegment {
                name: "utf8_is_word_byte_table".into(),
                data: Self::UTF8_IS_WORD_BYTE_LUT
                    .into_iter()
                    .map(|b| b as u8)
                    .collect(),
                position: table_pos,
            });

            Some(IsWordByteTable {
                position: table_pos.try_into().expect("position should fit in u64"),
            })
        } else {
            None
        };

        Ok((overall, Self { is_word_byte_table }))
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
    pub fn new(ctx: &mut CompileContext, layout: &LookLayout) -> Result<Self, BuildError> {
        let mut look_matches = [None; Self::NUM_LOOKS];
        let look_set = modified_lookset_for_dependencies(&ctx.nfa);

        if look_set.is_empty() {
            return Ok(Self { look_matches });
        }

        for look in look_set.iter() {
            if is_unsupported_lookaround(look) {
                // TODO: Need to implement the rest of the lookaround assertions
                return Err(BuildError::unsupported(format!(
                    "{look:?}/{} is not yet implemented",
                    look.as_char()
                )));
            }

            let sig = lookaround_fn_signature(lookaround_fn_name(look));

            let func = ctx.declare_function(sig);
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
                _ => unreachable!("Should be unreachable due to first loop through lookset"),
            };

            ctx.define_function(
                func_idx.expect("should have generated function for supported lookaround"),
                func_def,
            );
        }

        Ok(Self { look_matches })
    }

    /// TODO: Write docs for item
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

        FunctionDefinition {
            body,
            locals_name_map,
            labels_name_map: None,
            branch_hints: None,
        }
    }

    fn is_word_ascii_fn(is_word_byte_table: &IsWordByteTable) -> FunctionDefinition {
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

    fn is_word_start_ascii_fn(is_word_byte_table: &IsWordByteTable) -> FunctionDefinition {
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

    fn is_word_end_ascii_fn(is_word_byte_table: &IsWordByteTable) -> FunctionDefinition {
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

    fn is_word_start_half_ascii_fn(is_word_byte_table: &IsWordByteTable) -> FunctionDefinition {
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

    fn is_word_end_half_ascii_fn(is_word_byte_table: &IsWordByteTable) -> FunctionDefinition {
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

    fn word_before_ascii_instructions(
        instructions: &mut InstructionSink,
        is_word_byte_table: &IsWordByteTable,
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
            .i32_const(false as i32)
            .else_()
            // word_before = is_word_byte_table[haystack_ptr[at_offset - 1]];
            .local_get(2)
            .i64_const(1)
            .i64_sub()
            .local_get(0)
            .i64_add()
            .i32_load8_u(MemArg {
                offset: 0,
                align: 0,        // byte alignement
                memory_index: 0, // haystack
            })
            .i64_extend_i32_u()
            .i32_load8_u(MemArg {
                offset: is_word_byte_table.position,
                align: 0, // byte alignment
                memory_index: 1,
            })
            .end();
    }

    fn word_after_ascii_instructions(
        instructions: &mut InstructionSink,
        is_word_byte_table: &IsWordByteTable,
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
            .i32_const(false as i32)
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
                offset: is_word_byte_table.position,
                align: 0, // byte alignment
                memory_index: 1,
            })
            .end();
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

fn is_unsupported_lookaround(look: Look) -> bool {
    matches!(
        look,
        Look::WordUnicode
            | Look::WordUnicodeNegate
            | Look::WordStartUnicode
            | Look::WordEndUnicode
            | Look::WordStartHalfUnicode
            | Look::WordEndHalfUnicode
    )
}

fn needs_is_word_byte_lut(look_set: LookSet) -> bool {
    look_set.contains(Look::WordAscii)
        || look_set.contains(Look::WordAsciiNegate)
        || look_set.contains(Look::WordStartAscii)
        || look_set.contains(Look::WordEndAscii)
        || look_set.contains(Look::WordStartHalfAscii)
        || look_set.contains(Look::WordEndHalfAscii)
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
