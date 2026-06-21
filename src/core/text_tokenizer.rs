/// Tokenizes text into lowercase alphanumeric terms and reports character
/// offsets for callers that need positional data.
pub(crate) fn tokenize_each(text: &str, mut emit: impl FnMut(String, u32, u32)) {
    let mut term = String::new();
    let mut start = 0_u32;
    // Offsets are counted in Unicode scalar values to match `chars()` traversal.
    let mut current = 0_u32;

    for ch in text.chars() {
        if ch.is_alphanumeric() {
            if term.is_empty() {
                start = current;
            }
            for lowered in ch.to_lowercase() {
                term.push(lowered);
            }
        } else if !term.is_empty() {
            emit(std::mem::take(&mut term), start, current);
        }
        current += 1;
    }

    if !term.is_empty() {
        emit(term, start, current);
    }
}
