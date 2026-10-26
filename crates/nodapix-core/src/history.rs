//! Checkpointed Undo/Redo history stack.

#[derive(Clone, Debug)]
pub struct HistoryStack<T> {
    undo_stack: Vec<T>,
    redo_stack: Vec<T>,
    max_history: usize,
}

impl<T: Clone + PartialEq> Default for HistoryStack<T> {
    fn default() -> Self {
        Self::new(50)
    }
}

impl<T: Clone + PartialEq> HistoryStack<T> {
    pub fn new(max_history: usize) -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            max_history,
        }
    }

    /// Push a committed checkpoint state to the undo stack.
    /// Clears redo stack.
    pub fn push_checkpoint(&mut self, state: T) {
        if let Some(last) = self.undo_stack.last() {
            if last == &state {
                return;
            }
        }
        self.undo_stack.push(state);
        if self.undo_stack.len() > self.max_history {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    /// Undo: pops previous state from undo stack, pushes current to redo stack.
    pub fn undo(&mut self, current: T) -> Option<T> {
        let prev = self.undo_stack.pop()?;
        self.redo_stack.push(current);
        Some(prev)
    }

    /// Redo: pops future state from redo stack, pushes current to undo stack.
    pub fn redo(&mut self, current: T) -> Option<T> {
        let next = self.redo_stack.pop()?;
        self.undo_stack.push(current);
        Some(next)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }
}
