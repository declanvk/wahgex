#[cfg(feature = "wasmi")]
use regex_automata::Anchored;

/// This type is a mirror of [`regex_automata::Input`], with guaranteed
/// alignment and no-substructs.
#[derive(Debug)]
#[repr(C)]
#[cfg(feature = "wasmi")]
pub struct InputOpts {
    /// Whether to execute an "earliest" search or not.
    pub earliest: i32,
    /// Sets the anchor mode of a search.
    ///
    /// The translation:
    ///   - [`Anchored::No`] => `0`
    ///   - [`Anchored::Yes`] => `1`
    ///   - [`Anchored::Pattern`] => `2`
    pub anchored: i32,
    /// If `anchored` is equivalent to [`Anchored::Pattern`], then this is
    /// the
    /// [`PatternID`][regex_automata::util::primitives::PatternID].
    ///
    /// Otherwise, it is set to 0.
    pub anchored_pattern: i32,
}

#[cfg(feature = "wasmi")]
impl InputOpts {
    /// Creates a new `InputOpts` from a [`regex_automata::Input`].
    ///
    /// This translates the anchor mode and earliest flag into i32 values
    /// suitable for WASM.
    pub fn new(input: &regex_automata::Input<'_>) -> InputOpts {
        let (anchored, anchored_pattern) = match input.get_anchored() {
            Anchored::No => (0, 0),
            Anchored::Yes => (1, 0),
            Anchored::Pattern(id) => (2, i32::from_ne_bytes(id.to_ne_bytes())),
        };

        InputOpts {
            earliest: input.get_earliest() as i32,
            anchored,
            anchored_pattern,
        }
    }
}

/// This enum represents the results of the `prepare_input` function.
#[derive(Debug)]
#[cfg(feature = "compile")]
pub enum PrepareInputResult {
    /// Indicates that the input preparation was successful and no memory
    /// growth was needed.
    SuccessNoGrowth = 0,
    /// Indicates that the input preparation was successful and memory was
    /// grown to accommodate the haystack.
    SuccessGrowth = 1,
}
