//! Size caps

pub const TITLE_MAX: usize = 40;
pub const RENAME_MAX: usize = 80;
pub const TOOL_OUTPUT_MAX: usize = 20_000;
pub const MAX_OUT_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
/// An HTML widget (`show_widget`): one self-contained page, inlined whole into the webview's frame
pub const MAX_WIDGET_BYTES: u64 = 512 * 1024;
pub const EDIT_CONTEXT_MAX_BYTES: usize = 256 * 1024;
pub const PLAN_PREVIEW_MAX_BYTES: u64 = 1_048_576;
