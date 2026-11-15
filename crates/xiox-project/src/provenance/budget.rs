//! The shared allowance of one extraction: loop expansion, nesting depth and operations.

/// What one `extract` call may still spend, shared by every loop and nested shell inside it.
#[derive(Debug)]
pub struct Budget {
    nodes_left: usize,
    depth: usize,
}

impl Budget {
    /// Items of a loop that is expanded, as in the prototype.
    pub const MAX_LOOP_ITEMS: usize = 50;
    /// Nesting of expanded loops and nested shells together.
    pub const MAX_DEPTH: usize = 4;
    /// Operations one call may walk.
    pub const MAX_OPS: usize = 500;

    pub fn new() -> Self {
        Budget { nodes_left: Self::MAX_OPS, depth: 0 }
    }

    /// Spends one operation; false when none are left.
    pub fn take(&mut self) -> bool {
        self.nodes_left = match self.nodes_left.checked_sub(1) {
            Some(left) => left,
            None => return false,
        };
        true
    }

    /// Goes one level deeper; false, and no change, at the limit. Pair each success with [`Budget::leave`].
    pub fn enter(&mut self) -> bool {
        if self.depth >= Self::MAX_DEPTH {
            return false;
        }
        self.depth += 1;
        true
    }

    pub fn leave(&mut self) {
        self.depth -= 1;
    }
}

impl Default for Budget {
    fn default() -> Self {
        Self::new()
    }
}
