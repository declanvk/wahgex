use std::{
    alloc::{Layout, LayoutError},
    collections::{BTreeMap, VecDeque},
    sync::LazyLock,
};

use wasm_encoder::{BlockType, MemArg, NameMap, ValType};

use crate::{
    compile::{
        context::{
            ActiveDataSegment, CompileContext, Function, FunctionDefinition, FunctionIdx,
            FunctionSignature,
        },
        instructions::InstructionSinkExt,
        lookaround::{byte_word::IsWordByteLookupTable, perl_word::PERL_WORD},
    },
    util::repeat,
};
#[derive(Debug)]
struct PerlWordLookupTable {
    index: Vec<u8>,
    leaves: Vec<u8>,
}

#[expect(clippy::incompatible_msrv)]
static TABLE_INSTANCE: LazyLock<PerlWordLookupTable> = LazyLock::new(PerlWordLookupTable::new);

impl PerlWordLookupTable {
    const CHUNK: usize = 512 / 8;

    fn get() -> &'static Self {
        &TABLE_INSTANCE
    }

    // Stolen from `unicode-ident/generate/src/main.rs` @
    // 88c4aec1143fe452172f50879716a5b69cd4873c Author is David Tolnay
    fn new() -> Self {
        let mut chunkmap = BTreeMap::<[u8; Self::CHUNK], u8>::new();
        let mut dense = Vec::<[u8; Self::CHUNK]>::new();
        let mut new_chunk = |chunk| {
            if let Some(prev) = chunkmap.get(&chunk) {
                *prev
            } else {
                dense.push(chunk);
                let Ok(new) = u8::try_from(chunkmap.len()) else {
                    panic!("exceeded 256 unique chunks");
                };
                chunkmap.insert(chunk, new);
                new
            }
        };

        let properties: BTreeMap<_, _> = PERL_WORD
            .iter()
            .copied()
            .map(|(low, high)| (low, (low..=high)))
            .collect();

        let empty_chunk = [0u8; Self::CHUNK];
        new_chunk(empty_chunk);

        let mut index = Vec::<u8>::new();
        for i in 0..(u32::from(char::MAX) + 1) / Self::CHUNK as u32 / 8 {
            let mut chunk_bits = empty_chunk;
            for (j, this) in chunk_bits.iter_mut().enumerate().take(Self::CHUNK) {
                for k in 0..8u32 {
                    let code = (i * Self::CHUNK as u32 + j as u32) * 8 + k;
                    if code >= 0x80 {
                        if let Some(ch) = char::from_u32(code) {
                            let is_word = properties
                                .range(..=ch)
                                .next_back()
                                .map(|(_, range)| range.contains(&ch))
                                .unwrap_or(false);
                            *this |= (is_word as u8) << k;
                        }
                    }
                }
            }
            index.push(new_chunk(chunk_bits));
        }

        while let Some(0) = index.last() {
            index.pop();
        }

        let mut halfchunkmap = BTreeMap::new();
        for chunk in &dense {
            let mut front = [0u8; Self::CHUNK / 2];
            let mut back = [0u8; Self::CHUNK / 2];
            front.copy_from_slice(&chunk[..Self::CHUNK / 2]);
            back.copy_from_slice(&chunk[Self::CHUNK / 2..]);
            halfchunkmap
                .entry(front)
                .or_insert_with(VecDeque::new)
                .push_back(back);
        }

        let mut halfdense = Vec::<u8>::new();
        let mut dense_to_halfdense = BTreeMap::<u8, u8>::new();
        for chunk in &dense {
            let original_pos = chunkmap[chunk];
            if dense_to_halfdense.contains_key(&original_pos) {
                continue;
            }
            let mut front = [0u8; Self::CHUNK / 2];
            let mut back = [0u8; Self::CHUNK / 2];
            front.copy_from_slice(&chunk[..Self::CHUNK / 2]);
            back.copy_from_slice(&chunk[Self::CHUNK / 2..]);
            dense_to_halfdense.insert(
                original_pos,
                match u8::try_from(halfdense.len() / (Self::CHUNK / 2)) {
                    Ok(byte) => byte,
                    Err(_) => panic!("exceeded 256 half-chunks"),
                },
            );
            halfdense.extend_from_slice(&front);
            halfdense.extend_from_slice(&back);
            while let Some(next) = halfchunkmap.get_mut(&back).and_then(VecDeque::pop_front) {
                let mut concat = empty_chunk;
                concat[..Self::CHUNK / 2].copy_from_slice(&back);
                concat[Self::CHUNK / 2..].copy_from_slice(&next);
                let original_pos = chunkmap[&concat];
                if dense_to_halfdense.contains_key(&original_pos) {
                    continue;
                }
                dense_to_halfdense.insert(
                    original_pos,
                    match u8::try_from(halfdense.len() / (Self::CHUNK / 2) - 1) {
                        Ok(byte) => byte,
                        Err(_) => panic!("exceeded 256 half-chunks"),
                    },
                );
                halfdense.extend_from_slice(&next);
                back = next;
            }
        }

        for index in &mut index {
            *index = dense_to_halfdense[index];
        }

        Self {
            index,
            leaves: halfdense,
        }
    }

    // Also stolen from `unicode-ident/src/lib.rs` @
    // 88c4aec1143fe452172f50879716a5b69cd4873c Author is David Tolnay
    #[cfg(test)]
    fn is_word_character_test(&self, ch: char) -> bool {
        if ch.is_ascii() {
            return matches!(ch, '_' | '0'..='9' | 'a'..='z' | 'A'..='Z');
        }
        let chunk = self
            .index
            .get(ch as usize / 8 / Self::CHUNK)
            .copied()
            .unwrap_or(0);
        let offset = chunk as usize * Self::CHUNK / 2 + ch as usize / 8 % Self::CHUNK;
        unsafe { self.leaves.get_unchecked(offset) }.wrapping_shr(ch as u32 % 8) & 1 != 0
    }
}

/// TODO: Write docs for this item
#[derive(Debug)]
pub struct PerlWordLayout {
    index_table_position: u64,
    index_table_len: u64,
    leaves_table_position: u64,

    utf8_decode_classes_table_position: u64,
    utf8_decode_states_forward_table_position: u64,
}

impl PerlWordLayout {
    const ACCEPT: u32 = 12;
    /// TODO: Write docs for this item
    #[rustfmt::skip]
    const CLASSES: [u8; 256] = [
        0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,  0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
        0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,  0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
        0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,  0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
        0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,  0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
        1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,  9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,
        7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,  7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,
        8,8,2,2,2,2,2,2,2,2,2,2,2,2,2,2,  2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,
       10,3,3,3,3,3,3,3,3,3,3,3,3,4,3,3, 11,6,6,6,5,8,8,8,8,8,8,8,8,8,8,8,
    ];
    const REJECT: u32 = 0;
    /// TODO: Write docs for this item
    #[rustfmt::skip]
    const STATES_FORWARD: [u8; 108] = [
         0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,
        12,  0, 24, 36, 60, 96, 84,  0,  0,  0, 48, 72,
         0, 12,  0,  0,  0,  0,  0, 12,  0, 12,  0,  0,
         0, 24,  0,  0,  0,  0,  0, 24,  0, 24,  0,  0,
         0,  0,  0,  0,  0,  0,  0, 24,  0,  0,  0,  0,
         0, 24,  0,  0,  0,  0,  0,  0,  0, 24,  0,  0,
         0,  0,  0,  0,  0,  0,  0, 36,  0, 36,  0,  0,
         0, 36,  0,  0,  0,  0,  0, 36,  0, 36,  0,  0,
         0, 36,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,
    ];

    /// TODO: Write docs for this item
    pub fn new(
        ctx: &mut CompileContext,
        mut overall: Layout,
    ) -> Result<(Layout, Self), LayoutError> {
        let table = PerlWordLookupTable::get();

        let index_table_position: u64 = {
            let (table_index_layout, _table_stride) =
                repeat(&Layout::new::<u8>(), table.index.len())?;
            let (new_overall, table_pos) = overall.extend(table_index_layout)?;
            overall = new_overall;

            ctx.sections.add_active_data_segment(ActiveDataSegment {
                name: "utf8_is_word_character_index_table".into(),
                position: table_pos,
                data: table.index.clone(),
            });

            table_pos.try_into().expect("position should fit in u64")
        };

        let leaves_table_position: u64 = {
            let (table_leaves_layout, _table_stride) =
                repeat(&Layout::new::<u8>(), table.leaves.len())?;
            let (new_overall, table_pos) = overall.extend(table_leaves_layout)?;
            overall = new_overall;

            ctx.sections.add_active_data_segment(ActiveDataSegment {
                name: "utf8_is_word_character_leaves_table".into(),
                position: table_pos,
                data: table.leaves.clone(),
            });

            table_pos.try_into().expect("position should fit in u64")
        };

        let utf8_decode_classes_table_position = {
            let (table_classes_layout, _table_stride) =
                repeat(&Layout::new::<u8>(), Self::CLASSES.len())?;
            let (new_overall, table_pos) = overall.extend(table_classes_layout)?;
            overall = new_overall;

            ctx.sections.add_active_data_segment(ActiveDataSegment {
                name: "utf8_decode_classes_table".into(),
                position: table_pos,
                data: Self::CLASSES.into(),
            });

            table_pos.try_into().expect("position should fit in u64")
        };

        let utf8_decode_states_forward_table_position = {
            let (table_classes_layout, _table_stride) =
                repeat(&Layout::new::<u8>(), Self::STATES_FORWARD.len())?;
            let (new_overall, table_pos) = overall.extend(table_classes_layout)?;
            overall = new_overall;

            ctx.sections.add_active_data_segment(ActiveDataSegment {
                name: "utf8_decode_states_forward_table".into(),
                position: table_pos,
                data: Self::STATES_FORWARD.into(),
            });

            table_pos.try_into().expect("position should fit in u64")
        };

        let table = Self {
            index_table_position,
            index_table_len: table.index.len().try_into().expect("len should fit in u64"),
            leaves_table_position,
            utf8_decode_classes_table_position,
            utf8_decode_states_forward_table_position,
        };

        Ok((overall, table))
    }
}

/// TODO: Write docs for this item
#[derive(Debug)]
pub struct PerlWordFunctions {
    pub is_word_character: FunctionIdx,
    pub is_word_char_rev: FunctionIdx,
    pub is_word_char_fwd: FunctionIdx,
    pub decode_next_character: FunctionIdx,
    pub decode_last_character: FunctionIdx,
}

impl PerlWordFunctions {
    /// TODO: Write docs for this item
    pub const INVALID_CHAR: u32 = {
        let invalid = char::MAX as u32 + 1;
        assert!(char::from_u32(invalid).is_none());
        invalid
    };

    /// TODO: Write docs for this item
    pub fn new(
        ctx: &mut CompileContext,
        layout: &PerlWordLayout,
        is_word_byte_table: &IsWordByteLookupTable,
    ) -> Self {
        // TODO(opt): Make the generation of each function conditional on
        // specific lookahead states

        let is_word_character =
            ctx.add_function(Self::is_word_character_fn(layout, is_word_byte_table));

        let decode_next_character = ctx.add_function(Self::decode_next_character_fn(layout));

        let decode_last_character =
            ctx.add_function(Self::decode_last_character_fn(decode_next_character));

        let is_word_char_rev = ctx.add_function(Self::is_word_char_rev_fn(
            decode_last_character,
            is_word_character,
        ));

        let is_word_char_fwd = ctx.add_function(Self::is_word_char_fwd_fn(
            decode_next_character,
            is_word_character,
        ));

        Self {
            is_word_character,
            is_word_char_rev,
            is_word_char_fwd,
            decode_next_character,
            decode_last_character,
        }
    }

    fn is_word_character_fn(
        layout: &PerlWordLayout,
        is_word_byte_table: &IsWordByteLookupTable,
    ) -> Function {
        let mut locals_name_map = NameMap::new();
        // Parameters
        locals_name_map.append(0, "character");
        // Locals
        locals_name_map.append(1, "chunk");
        locals_name_map.append(2, "index_offset");
        let mut labels_name_map = NameMap::new();
        labels_name_map.append(0, "ascii_base_case");
        labels_name_map.append(1, "lookup_chunk");

        // Sketch:
        // ```rust
        // if character <= 0x7F {
        //     return utf8_is_word_byte_table[character]
        // }
        //
        // chunk = 0
        // index_offset = character / 8 / CHUNK
        // if index_offset < index.len() {
        //     chunk = utf8_is_word_character_index_table[index_offset]
        // }
        //
        // offset = chunk * CHUNK / 2 + character / 8 % CHUNK;
        //
        // return (utf8_is_word_character_leaves_table[offset] >> (character % 8)) & 1 != 0
        // ```

        let mut body = wasm_encoder::Function::new([(1, ValType::I32), (1, ValType::I64)]);

        body.instructions()
            // if character <= 0x7F {
            .local_get(0)
            .i32_const(0x7F)
            .i32_le_u()
            .if_(BlockType::Empty)
            //     return utf8_is_word_byte_table[character]
            .local_get(0)
            .i64_extend_i32_u()
            .i32_load8_u(MemArg {
                offset: is_word_byte_table.position(),
                align: 0, // byte alignment
                memory_index: 1,
            })
            .return_()
            // } - end if
            .end()
            // chunk = 0 - implicit, locals are zero initialized
            // index_offset = character / 8 / CHUNK
            .local_get(0)
            .i32_const({
                debug_assert!(
                    PerlWordLookupTable::CHUNK.is_power_of_two(),
                    "PerlWordLookupTable::CHUNK must be a power of 2"
                );

                let shift = 8u32.ilog2() + PerlWordLookupTable::CHUNK.ilog2();
                i32::from_ne_bytes(shift.to_ne_bytes())
            })
            .i32_shr_u()
            .i64_extend_i32_u()
            .local_tee(2)
            // if character < utf8_is_word_character_index_table.len() {
            .u64_const(layout.index_table_len)
            .i64_lt_u()
            .if_(BlockType::Empty)
            .local_get(2)
            //     chunk = utf8_is_word_character_index_table[index_offset]
            .i32_load8_u(MemArg {
                offset: layout.index_table_position,
                align: 0, // byte alignment
                memory_index: 1,
            })
            .local_set(1)
            // } - end if
            .end()
            // offset = chunk * CHUNK / 2 + character / 8 % CHUNK;
            .local_get(1)
            .u32_const(u32::try_from(PerlWordLookupTable::CHUNK).expect("chunk should fit in u32"))
            .i32_mul()
            .i32_const(1) // log_2(2)
            .i32_shr_u()
            .local_get(0)
            .i32_const(3) // log_2(8)
            .i32_shr_u()
            .u32_const(u32::try_from(PerlWordLookupTable::CHUNK).expect("chunk should fit in u32"))
            .i32_rem_u()
            .i32_add()
            // return (utf8_is_word_character_leaves_table[offset] >> (character % 8)) & 1 != 0
            .i64_extend_i32_u()
            .i32_load8_u(MemArg {
                offset: layout.leaves_table_position,
                align: 0, // byte alignment
                memory_index: 1,
            })
            .local_get(0)
            .i32_const(8)
            .i32_rem_u()
            .i32_shr_u()
            .i32_const(1)
            .i32_and()
            .end();

        Function {
            sig: FunctionSignature {
                name: "utf8_is_word_character".into(),
                // [character]
                params_ty: &[ValType::I32],
                // [is_word]
                results_ty: &[ValType::I32],
                export: false,
            },
            def: FunctionDefinition {
                body,
                locals_name_map,
                labels_name_map: Some(labels_name_map),
                branch_hints: None,
            },
        }
    }

    // This implementation is ported from the `bstr` crate, specifically looking at
    // `bstr/src/utf8.rs` @ 955fa1609eefb23fa3d324db1e57781f33b8fe3c. Author is
    // primarily Andrew Gallant. Licensed MIT & Apache
    fn decode_next_character_fn(layout: &PerlWordLayout) -> Function {
        let mut locals_name_map = NameMap::new();
        // Parameters
        locals_name_map.append(0, "haystack_slice_ptr");
        locals_name_map.append(1, "haystack_slice_len");
        // Locals
        locals_name_map.append(2, "byte");
        locals_name_map.append(3, "state");
        locals_name_map.append(4, "code_point");
        locals_name_map.append(5, "index");
        locals_name_map.append(6, "class");
        let mut labels_name_map = NameMap::new();
        labels_name_map.append(0, "check_empty_slice");
        labels_name_map.append(1, "check_ascii_byte");
        labels_name_map.append(2, "grow_code_point_loop");
        labels_name_map.append(3, "check_code_point_loop_condition");
        labels_name_map.append(4, "check_accept_state_update_code_point");
        labels_name_map.append(5, "check_accept_state_return");
        labels_name_map.append(6, "check_reject_state_return");

        // Sketch:
        // ```rust
        // if slice_len == 0 {
        //     return (INVALID_CHAR, 0)
        // }
        //
        // let byte = slice_ptr[0]
        // if byte <= 0x7F {
        //     return (byte, 1)
        // }
        //
        // let state = ACCEPT
        // let code_point = 0
        // let index = 0
        // loop {
        //     if index >= slice_len {
        //         break
        //     }
        //
        //     let byte = slice_ptr[index]
        //     let class = CLASSES[byte];
        //     if state == ACCEPT {
        //         code_point = (0xFF >> class) & byte;
        //     } else {
        //         code_point = (byte & 0b0011_1111) | (code_point << 6);
        //     }
        //     state = STATES_FORWARD[state + class];
        //
        //     index += 1;
        //
        //     if state == ACCEPT {
        //         return (code_point, index);
        //     } else if state == REJECT {
        //         return (INVALID_CHAR, cmp::max(1, index.saturating_sub(1)));
        //     }
        // }
        // return (INVALID_CHAR, i)
        // ```

        let mut body =
            wasm_encoder::Function::new([(3, ValType::I32), (1, ValType::I64), (1, ValType::I32)]);

        body.instructions()
            // if slice_len == 0 {
            .local_get(1)
            .i64_eqz()
            .if_(BlockType::Empty)
            //     return (INVALID_CHAR, 0)
            .u32_const(Self::INVALID_CHAR)
            .i64_const(0)
            .return_()
            // } - end if
            .end()
            // let byte = slice_ptr[0]
            .local_get(0)
            .i32_load8_u(MemArg {
                offset: 0,
                align: 0,
                // loading from haystack memory
                memory_index: 0,
            })
            .local_tee(2)
            // if byte <= 0x7F {
            .i32_const(0x7F)
            .i32_le_u()
            .if_(BlockType::Empty)
            //     return (byte, 1)
            .local_get(2)
            .u64_const(1)
            .return_()
            // } - end if
            .end()
            // let state = ACCEPT
            // let code_point = 0
            // let index = 0
            .u32_const(PerlWordLayout::ACCEPT)
            .local_set(3)
            // other locals are already zero-inited
            // loop {
            .block(BlockType::Empty) // block needed for loop break
            .loop_(BlockType::Empty)
            //     if index >= slice_len {
            //         break
            //     }
            .local_get(5)
            .local_get(1)
            .i64_ge_u()
            .br_if(1)
            //     let byte = slice_ptr[index]
            .local_get(0)
            .local_get(5)
            .i64_add()
            .i32_load8_u(MemArg {
                offset: 0,
                align: 0,
                // loading from haystack memory
                memory_index: 0,
            })
            .local_tee(2)
            //     let class = CLASSES[byte];
            .i64_extend_i32_u()
            .i32_load8_u(MemArg {
                offset: layout.utf8_decode_classes_table_position,
                align: 0,
                // load from state memory
                memory_index: 1,
            })
            .local_set(6)
            //     if state == ACCEPT {
            .local_get(3)
            .u32_const(PerlWordLayout::ACCEPT)
            .i32_eq()
            .if_(BlockType::Result(ValType::I32))
            //         (0xFF >> class) & byte;
            .i32_const(0xFF)
            .local_get(6)
            .i32_shr_u()
            .local_get(2)
            .i32_and()
            .else_()
            //         (byte & 0b0011_1111) | (code_point << 6);
            .local_get(2)
            .i32_const(0b0011_1111)
            .i32_and()
            .local_get(4)
            .u32_const(6)
            .i32_shl()
            .i32_or()
            //     } - end if-else
            .end()
            //         code_point = ...
            .local_set(4)
            //     state = STATES_FORWARD[state + class];
            .local_get(3)
            .local_get(6)
            .i32_add()
            .i64_extend_i32_u()
            .i32_load8_u(MemArg {
                offset: layout.utf8_decode_states_forward_table_position,
                align: 0,
                // load from state memory
                memory_index: 1,
            })
            .local_set(3)
            //     index += 1;
            .local_get(5)
            .u64_const(1)
            .i64_add()
            .local_set(5)
            //     if state == ACCEPT {
            .local_get(3)
            .u32_const(PerlWordLayout::ACCEPT)
            .i32_eq()
            .if_(BlockType::Empty)
            //         return (code_point, index);
            .local_get(4)
            .local_get(5)
            .return_()
            .else_()
            //     } else if state == REJECT {
            .local_get(3)
            .u32_const(PerlWordLayout::REJECT)
            .i32_eq()
            .if_(BlockType::Empty)
            //         return (INVALID_CHAR, cmp::max(1, index.saturating_sub(1)));
            .u32_const(Self::INVALID_CHAR)
            // core::cmp::max(1, index.saturating_sub(1)) translates to
            // saturating_sub:
            //   local.get 0
            //   i32.const 2
            //   local.get 0
            //   i32.const 2
            //   i32.gt_u
            //   i32.select
            //   i32.const -1
            //   i32.add
            //   end_function
            .local_get(5)
            .u64_const(2)
            .local_get(5)
            .u64_const(2)
            .i64_gt_u()
            .select()
            .i64_const(-1)
            .i64_add()
            .return_()
            //     }
            .end() // end else-if
            .end() // end if
            // } - end/continue loop
            .br(0)
            .end()
            .end() // end break block
            //         return (INVALID_CHAR, i)
            .u32_const(Self::INVALID_CHAR)
            .local_get(5)
            .end();

        Function {
            sig: FunctionSignature {
                name: "utf8_decode_next_character".into(),
                params_ty: &[ValType::I64, ValType::I64],
                results_ty: &[ValType::I32, ValType::I64],
                export: false,
            },
            def: FunctionDefinition {
                body,
                locals_name_map,
                labels_name_map: Some(labels_name_map),
                branch_hints: None,
            },
        }
    }

    fn decode_last_character_fn(decode_next_character: FunctionIdx) -> Function {
        let mut locals_name_map = NameMap::new();
        // Parameters
        locals_name_map.append(0, "haystack_slice_ptr");
        locals_name_map.append(1, "haystack_slice_len");
        // Locals
        locals_name_map.append(2, "start");
        locals_name_map.append(3, "limit");
        locals_name_map.append(4, "size");
        locals_name_map.append(5, "character");
        let mut labels_name_map = NameMap::new();
        labels_name_map.append(0, "check_empty_slice");
        labels_name_map.append(1, "find_start_loop");
        labels_name_map.append(2, "check_start_loop_condition");
        labels_name_map.append(3, "check_decode_result");

        // Sketch:
        // ```rust
        // if slice_len == 0 {
        //     return (None, 0);
        // }
        //
        // start = slice_len - 1;
        // limit = slice_len.saturating_sub(4);
        // loop {
        //     if start <= limit || ((slice_ptr[start] & 0b1100_0000) != 0b1000_0000) {
        //         break;
        //     }
        //     start -= 1;
        // }
        // let (ch, size) = decode_next_character(slice_ptr + start, slice_len - start);
        //
        // if start + size != slice_len {
        //     return (None, 1)
        // }
        // return (ch, size)
        // ```

        // Aux sketch (`a` and `b` are both `u32`):
        // ```rust
        // a.saturating_sub(b)
        // ```
        // translates to
        // ```wasm
        // saturating_sub:
        //   i32.const 0
        //   local.get 0
        //   local.get 1
        //   i32.sub
        //   local.tee 1
        //   local.get 1
        //   local.get 0
        //   i32.gt_u
        //   i32.select
        //   end_function
        // ```

        let mut body = wasm_encoder::Function::new([(3, ValType::I64), (1, ValType::I32)]);

        body.instructions()
            // if slice_len == 0 {
            .local_get(1)
            .i64_eqz()
            .if_(BlockType::Empty)
            //     return (None, 0);
            .u32_const(Self::INVALID_CHAR)
            .i64_const(0)
            .return_()
            // } - end if
            .end()
            // start = slice_len - 1;
            .local_get(1)
            .u64_const(1)
            .i64_sub()
            .local_set(2)
            // limit = slice_len.saturating_sub(4);
            .i64_const(0)
            .local_get(1)
            .u64_const(4)
            .i64_sub()
            .local_tee(3) // using `limit` local as scratch
            .local_get(3)
            .local_get(1)
            .i64_gt_u()
            .select()
            .local_set(3)
            // loop {
            .block(BlockType::Empty) // block is needed for break target
            .loop_(BlockType::Empty)
            //     if start <= limit || ((slice_ptr[start] & 0b1100_0000) != 0b1000_0000) {
            //         break;
            //     }
            .local_get(2)
            .local_get(3)
            .i64_le_u()
            .local_get(2)
            .local_get(0)
            .i64_add()
            .i32_load8_u(MemArg {
                offset: 0,
                // loading a single byte
                align: 0,
                // from the haystack memory
                memory_index: 0,
            })
            .i32_const(0b1100_0000)
            .i32_and()
            .i32_const(0b1000_0000)
            .i32_ne()
            .i32_or()
            .br_if(1)
            //     start -= 1;
            .local_get(2)
            .u64_const(1)
            .i64_sub()
            .local_set(2)
            // } - end/continue loop & block
            .br(0)
            .end()
            .end()
            // let (ch, size) = decode_next_character(slice_ptr + start, slice_len - start);
            .local_get(0)
            .local_get(2)
            .i64_add()
            .local_get(1)
            .local_get(2)
            .i64_sub()
            .call(decode_next_character.into())
            // stack has [ch, size] <- top
            .local_set(4)
            .local_set(5)
            .local_get(4)
            // if start + size != slice_len {
            .local_get(2)
            .i64_add()
            .local_get(1)
            .i64_ne()
            .if_(BlockType::Empty)
            //     return (None, 1)
            .u32_const(Self::INVALID_CHAR)
            .u64_const(1)
            .return_()
            .else_()
            // } - end ifelse
            .end()
            // return (ch, size)
            .local_get(5)
            .local_get(4)
            .end();

        Function {
            sig: FunctionSignature {
                name: "utf8_decode_last_character".into(),
                params_ty: &[ValType::I64, ValType::I64],
                results_ty: &[ValType::I32, ValType::I64],
                export: false,
            },
            def: FunctionDefinition {
                body,
                locals_name_map,
                labels_name_map: Some(labels_name_map),
                branch_hints: None,
            },
        }
    }

    fn is_word_char_rev_fn(
        decode_last_character: FunctionIdx,
        is_word_character: FunctionIdx,
    ) -> Function {
        let mut locals_name_map = NameMap::new();
        // Parameters
        locals_name_map.append(0, "haystack_ptr");
        locals_name_map.append(1, "haystack_len");
        locals_name_map.append(2, "at_offset");
        // Locals
        locals_name_map.append(3, "character");
        let mut labels_name_map = NameMap::new();
        labels_name_map.append(0, "check_invalid_result_return");

        // Sketch:
        // ```rust
        // let (character, _) = utf8_decode_last_character(haystack_ptr, at_offset)
        //
        // if character != INVALID_CHAR {
        //     return is_word_character(character)
        // } else {
        //     return false
        // }
        // ```

        let mut body = wasm_encoder::Function::new([(1, ValType::I32)]);

        body.instructions()
            // let (character, _) = utf8_decode_last_character(haystack_ptr, at_offset)
            .local_get(0)
            .local_get(2)
            .call(decode_last_character.into())
            .drop()
            .local_tee(3)
            // if character != INVALID_CHAR {
            .u32_const(Self::INVALID_CHAR)
            .i32_ne()
            .if_(BlockType::Result(ValType::I32))
            //     return is_word_character(character)
            .local_get(3)
            .call(is_word_character.into())
            .else_()
            //     return false
            .bool_const(false)
            .end()
            .return_()
            .end();

        Function {
            sig: FunctionSignature {
                name: "utf8_is_word_char_rev".into(),
                // [haystack_ptr, haystack_len, at_offset]
                params_ty: &[ValType::I64, ValType::I64, ValType::I64],
                // [is_match]
                results_ty: &[ValType::I32],
                export: false,
            },
            def: FunctionDefinition {
                body,
                locals_name_map,
                labels_name_map: Some(labels_name_map),
                branch_hints: None,
            },
        }
    }

    fn is_word_char_fwd_fn(
        decode_next_character: FunctionIdx,
        is_word_character: FunctionIdx,
    ) -> Function {
        let mut locals_name_map = NameMap::new();
        // Parameters
        locals_name_map.append(0, "haystack_ptr");
        locals_name_map.append(1, "haystack_len");
        locals_name_map.append(2, "at_offset");
        // Locals
        locals_name_map.append(3, "character");

        // Sketch:
        // ```rust
        // let haystack_slice_ptr = haystack_ptr + at_offset
        // let haystack_slice_len = haystack_len - at_offset
        // let (character, _) = utf8_decode_next_character(haystack_slice_ptr, haystack_slice_len)
        //
        // if character != INVALID_CHAR {
        //     return is_word_character(character)
        // } else {
        //     return false
        // }
        // ```

        let mut body = wasm_encoder::Function::new([(1, ValType::I32)]);

        body.instructions()
            // let haystack_slice_ptr = haystack_ptr + at_offset
            .local_get(0)
            .local_get(2)
            .i64_add()
            // let haystack_slice_len = haystack_len - at_offset
            .local_get(1)
            .local_get(2)
            .i64_sub()
            // let (character, _) = utf8_decode_next_character(.., ..)
            .call(decode_next_character.into())
            .drop()
            .local_tee(3)
            // if character != INVALID_CHAR {
            .u32_const(Self::INVALID_CHAR)
            .i32_ne()
            .if_(BlockType::Result(ValType::I32))
            //     return is_word_character(character)
            .local_get(3)
            .call(is_word_character.into())
            .else_()
            //     return false
            .bool_const(false)
            .end()
            .return_()
            .end();

        Function {
            sig: FunctionSignature {
                name: "utf8_is_word_char_fwd".into(),
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

#[cfg(test)]
mod tests {
    use std::mem;

    use regex_automata::nfa::thompson::NFA;

    use crate::{compile::tests::setup_interpreter, Config};

    use super::*;

    #[test]
    fn transformed_table_size() {
        let table = PerlWordLookupTable::get();
        assert_eq!(table.index.len(), 1793);
        assert_eq!(table.leaves.len(), 5056);
        // 1793 + 5056 = 6849
    }

    #[test]
    fn perl_word_ranges_have_no_gaps() {
        for (low, high) in PERL_WORD.iter().copied() {
            let dist = u32::from(high) - u32::from(low) + 1;
            let size = (low..=high).count() as u32;
            assert_eq!(
                dist, size,
                "Distance not equal to size for range [{low}, {high}]"
            )
        }
    }

    #[test]
    fn original_table_size() {
        let num_chars: u32 = PERL_WORD
            .iter()
            .map(|(low, high)| u32::from(*high) - u32::from(*low))
            .sum();

        assert_eq!(num_chars, 148564);
        assert_eq!(mem::size_of_val(PERL_WORD), 6416);
    }

    struct TestSetup {
        _engine: wasmi::Engine,
        _module: wasmi::Module,
        store: wasmi::Store<()>,
        instance: wasmi::Instance,
    }

    impl TestSetup {
        fn setup() -> Self {
            let cfg = Config::new().export_all_functions(true).export_state(true);
            let mut ctx = CompileContext::new(NFA::always_match(), cfg);
            let overall = Layout::new::<()>();
            let (overall, is_word_byte_table) =
                IsWordByteLookupTable::new(&mut ctx, overall).unwrap();
            let (overall, perl_world_table_layout) =
                PerlWordLayout::new(&mut ctx, overall).unwrap();
            let _funcs =
                PerlWordFunctions::new(&mut ctx, &perl_world_table_layout, &is_word_byte_table);

            assert_eq!(overall.align(), 1);
            assert_eq!(overall.size(), 7469);

            let module = ctx.compile(&overall);
            let module_bytes = module.finish();
            let (engine, module, store, instance) = setup_interpreter(&module_bytes);

            Self {
                _engine: engine,
                _module: module,
                store,
                instance,
            }
        }
    }

    #[test]
    fn is_word_character() {
        let mut setup = TestSetup::setup();
        let utf8_is_word_character = setup
            .instance
            .get_typed_func::<(i32,), i32>(&setup.store, "utf8_is_word_character")
            .unwrap();

        fn char_to_i32(c: char) -> i32 {
            i32::from_ne_bytes(u32::from(c).to_ne_bytes())
        }

        let perl_word_table = PerlWordLookupTable::get();

        let mut check = |c: char, expected: bool| {
            let res = utf8_is_word_character
                .call(&mut setup.store, (char_to_i32(c),))
                .unwrap();
            let non_wasm_is_word = perl_word_table.is_word_character_test(c);
            let is_word = res != 0;
            assert_eq!(
                is_word,
                expected,
                "Tested '{c}' as word character. Non-wasm lookup says '{c}' is {non_wasm_is_word} \
                 a word. Integer value {}",
                u32::from(c)
            );
            assert_eq!(
                non_wasm_is_word, is_word,
                "Checking '{c}' against non-wasm lookup"
            );
        };

        for (low, high) in PERL_WORD.iter().copied() {
            if let Ok(char) = char::try_from(low as u32 - 1) {
                check(char, false);
            }
            check(low, true);

            if low != high {
                // This midpoint should be within the range of a perl character
                let mid = (high as u32 - low as u32) / 2 + low as u32;
                check(char::try_from(mid).expect("should be valid char"), true);
                check(high, true);
            }

            if let Ok(char) = char::try_from(high as u32 + 1) {
                check(char, false);
            }
        }

        check('α', true);
        check('β', true);
    }

    #[test]
    fn is_word_char_rev_and_fwd() {
        let mut setup = TestSetup::setup();

        let haystack_mem = setup.instance.get_memory(&setup.store, "haystack").unwrap();
        let is_word_char_fwd = setup
            .instance
            .get_typed_func::<(i64, i64, i64), i32>(&setup.store, "utf8_is_word_char_fwd")
            .unwrap();
        let is_word_char_rev = setup
            .instance
            .get_typed_func::<(i64, i64, i64), i32>(&setup.store, "utf8_is_word_char_rev")
            .unwrap();

        let mut run_test = |haystack: &[u8], expected_fwd: &[bool], expected_rev: &[bool]| {
            haystack_mem.write(&mut setup.store, 0, haystack).unwrap();
            let haystack_len = haystack.len() as i64;

            for i in 0..=haystack.len() {
                let at = i as i64;
                let res_rev = is_word_char_rev
                    .call(&mut setup.store, (0, haystack_len, at))
                    .unwrap();
                assert_eq!(
                    res_rev != 0,
                    expected_rev[i],
                    "rev check failed for '{:?}' at index {}",
                    String::from_utf8_lossy(haystack),
                    i
                );

                let res_fwd = is_word_char_fwd
                    .call(&mut setup.store, (0, haystack_len, at))
                    .unwrap();
                assert_eq!(
                    res_fwd != 0,
                    expected_fwd[i],
                    "fwd check failed for '{:?}' at index {}",
                    String::from_utf8_lossy(haystack),
                    i
                );
            }

            haystack_mem.data_mut(&mut setup.store)[..haystack.len()].fill(0); // clear previous test memory
        };

        // Haystack: `a b`
        run_test(
            b"a b",
            // fwd: a,  , b, EOF
            &[true, false, true, false],
            // rev: BOS, a,  , b
            &[false, true, false, true],
        );

        // Haystack: `α β` (alpha, space, beta)
        run_test(
            "α β".as_bytes(),
            // fwd: α, INVAL,  , β, INVAL, EOF
            &[true, false, false, true, false, false],
            // rev: BOS, INVAL, α,  , INVAL, β
            &[false, false, true, false, false, true],
        );

        // Haystack: `aαb`
        run_test(
            "aαb".as_bytes(),
            // fwd: a, α, INVAL, b, EOF
            &[true, true, false, true, false],
            // rev: BOS, a, INVAL, α, b
            &[false, true, false, true, true],
        );

        // Haystack: `_`
        run_test(
            b"_",
            // fwd: _, EOF
            &[true, false],
            // rev: BOS, _
            &[false, true],
        );

        // Haystack: `` (empty)
        run_test(
            b"",
            // fwd: EOF
            &[false],
            // rev: BOS
            &[false],
        );

        // Haystack: invalid utf8
        run_test(
            b"\xff\xfe",
            // fwd: INVAL, INVAL, EOF
            &[false, false, false],
            // rev: BOS, INVAL, INVAL
            &[false, false, false],
        );
    }
}
