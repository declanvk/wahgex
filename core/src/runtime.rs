//! TODO: Write docs for this item

/// TODO: Write docs for this item
#[derive(Debug, zerocopy::KnownLayout, zerocopy::Immutable, zerocopy::FromZeros)]
#[repr(C)]
pub struct MatchResultHeader {
    /// Set to `true` if the regex found a match in the haystack.
    ///
    /// If `false`, other fields in this struct may not be valid.
    pub is_match: bool,
}

#[cfg(test)]
mod tests {
    use std::alloc::Layout;

    use super::*;

    #[test]
    fn run_result_layout() {
        assert_eq!(
            Layout::new::<MatchResultHeader>(),
            Layout::from_size_align(1, 1).unwrap()
        )
    }
}
