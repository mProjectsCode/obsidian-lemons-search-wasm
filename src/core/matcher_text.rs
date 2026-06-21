use nucleo_matcher::{Utf32Str, Utf32String};

/// Searchable text stored in the representation expected by `nucleo_matcher`.
pub(crate) struct MatcherText {
    utf32: Utf32String,
}

impl MatcherText {
    /// Converts a Rust string into the UTF-32 representation expected by
    /// `nucleo_matcher`.
    pub(crate) fn new(string: String) -> Self {
        MatcherText {
            utf32: Utf32String::from(string),
        }
    }

    #[inline]
    /// Borrows the matcher-ready UTF-32 string.
    pub(crate) fn as_utf32_str(&self) -> Utf32Str<'_> {
        self.utf32.slice(..)
    }
}
