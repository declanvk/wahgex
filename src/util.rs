use std::alloc::{Layout, LayoutError};

const DEFAULT_ERROR: LayoutError = const {
    match Layout::from_size_align(0, 3) {
        Ok(_) => panic!("expected err"),
        Err(err) => err,
    }
};

pub const fn repeat(layout: &Layout, n: usize) -> Result<(Layout, usize), LayoutError> {
    let padded = layout.pad_to_align();
    if let Ok(repeated) = repeat_packed(&padded, n) {
        Ok((repeated, padded.size()))
    } else {
        Err(DEFAULT_ERROR)
    }
}

const fn repeat_packed(layout: &Layout, n: usize) -> Result<Layout, LayoutError> {
    if let Some(size) = layout.size().checked_mul(n) {
        // The safe constructor is called here to enforce the isize size limit.
        Layout::from_size_align(size, layout.align())
    } else {
        Err(DEFAULT_ERROR)
    }
}
