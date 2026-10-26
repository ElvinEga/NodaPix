//! NodaPix Core: Document model, non-destructive editing state, and history.

pub mod document;
pub mod edit_state;
pub mod history;
pub mod pipeline;

pub use document::ImageDocument;
pub use edit_state::EditState;
pub use history::HistoryStack;
pub use pipeline::execute_pipeline;
