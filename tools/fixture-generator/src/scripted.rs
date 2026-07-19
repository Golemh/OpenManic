//! Generic scripted inputs and recorded outputs for deterministic tests.

use std::collections::VecDeque;

/// A finite, ordered input script.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScriptedInput<T> {
    pending: VecDeque<T>,
}

impl<T> ScriptedInput<T> {
    /// Creates an input script from values yielded in iterator order.
    pub fn new(values: impl IntoIterator<Item = T>) -> Self {
        Self {
            pending: values.into_iter().collect(),
        }
    }

    /// Returns how many scripted values have not yet been consumed.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.pending.len()
    }
}

impl<T> Iterator for ScriptedInput<T> {
    type Item = T;

    /// Returns the next scripted value, or `None` after the script is exhausted.
    fn next(&mut self) -> Option<Self::Item> {
        self.pending.pop_front()
    }
}

/// A generic sink that records values in write order for assertions.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecordingSink<T> {
    values: Vec<T>,
}

impl<T> RecordingSink<T> {
    /// Creates an empty recording sink.
    #[must_use]
    pub const fn new() -> Self {
        Self { values: Vec::new() }
    }

    /// Records one value.
    pub fn record(&mut self, value: T) {
        self.values.push(value);
    }

    /// Returns the values in their recorded order.
    #[must_use]
    pub fn values(&self) -> &[T] {
        &self.values
    }

    /// Consumes the sink and returns its recorded values.
    #[must_use]
    pub fn into_values(self) -> Vec<T> {
        self.values
    }
}

#[cfg(test)]
mod tests {
    use super::{RecordingSink, ScriptedInput};

    #[test]
    fn scripted_input_preserves_order_and_reports_exhaustion() {
        let mut input = ScriptedInput::new(["first", "second"]);
        assert_eq!(input.remaining(), 2);
        assert_eq!(input.next(), Some("first"));
        assert_eq!(input.next(), Some("second"));
        assert_eq!(input.next(), None);
        assert_eq!(input.remaining(), 0);
    }

    #[test]
    fn recording_sink_preserves_write_order() {
        let mut sink = RecordingSink::new();
        sink.record(3);
        sink.record(5);
        assert_eq!(sink.values(), [3, 5]);
        assert_eq!(sink.into_values(), vec![3, 5]);
    }
}
