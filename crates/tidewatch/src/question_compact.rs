//! What a click on the Overview's inline answer buttons means; it is answered through the same
//! function as the Reader's card.

/// What a click on an inline answer means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Click {
    /// The option at this place in the question's options.
    Option(usize),
    Chat,
    /// Answer it in the tab, where the full card is.
    Open,
}
