//! This module contains type for emitting WASM instructions specifically for
//! the regex machine.

use std::alloc::Layout;

use wasm_encoder::{InstructionSink, MemArg};

pub trait InstructionSinkExt {
    fn state_id_load(&mut self, offset: u64, state_id_layout: &Layout) -> &mut Self;

    fn state_id_store(&mut self, offset: u64, state_id_layout: &Layout) -> &mut Self;
}

impl InstructionSinkExt for InstructionSink<'_> {
    fn state_id_load(&mut self, offset: u64, state_id_layout: &Layout) -> &mut Self {
        let state_id_size = state_id_layout.size();
        if state_id_size == 1 {
            self.i32_load8_u(MemArg {
                offset,
                align: state_id_layout.align().ilog2(),
                memory_index: 1, // states are always stored in the state memory
            })
        } else if state_id_size == 2 {
            self.i32_load16_u(MemArg {
                offset,
                align: state_id_layout.align().ilog2(),
                memory_index: 1, // states are always stored in the state memory
            })
        } else {
            self.i32_load(MemArg {
                offset,
                align: state_id_layout.align().ilog2(),
                memory_index: 1, // states are always stored in the state memory
            })
        }
    }

    fn state_id_store(&mut self, offset: u64, state_id_layout: &Layout) -> &mut Self {
        let state_id_size = state_id_layout.size();
        let align = state_id_layout.align().ilog2();
        if state_id_size == 1 {
            self.i32_store8(MemArg {
                offset,
                align,
                memory_index: 1, // states are always stored in the state memory
            })
        } else if state_id_size == 2 {
            self.i32_store16(MemArg {
                offset,
                align,
                memory_index: 1, // states are always stored in the state memory
            })
        } else {
            self.i32_store(MemArg {
                offset,
                align,
                memory_index: 1, // states are always stored in the state memory
            })
        }
    }
}
