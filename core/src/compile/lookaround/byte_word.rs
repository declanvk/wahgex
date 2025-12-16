use std::alloc::{Layout, LayoutError};

use crate::compile::{
    context::{ActiveDataSegment, CompileContext},
    util::repeat,
};

#[derive(Debug)]
pub struct IsWordByteLookupTable {
    position: u64,
}

impl IsWordByteLookupTable {
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

    pub fn position(&self) -> u64 {
        self.position
    }

    /// TODO: Write docs for this item
    pub fn new(
        ctx: &mut CompileContext,
        mut overall: Layout,
    ) -> Result<(Layout, Self), LayoutError> {
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

        let table = Self {
            position: table_pos.try_into().expect("position should fit in u64"),
        };

        Ok((overall, table))
    }
}
