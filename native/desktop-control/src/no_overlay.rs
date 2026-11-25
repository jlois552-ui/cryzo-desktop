//! Outside Windows the overlay is driven through [`crate::motion::Indicator`]: the Linux overlay
//! draws in this process (`linux_overlay.rs`) and the macOS one in a helper process
//! (`mac_overlay.rs`). The browser tools take it under the same name as Windows' overlay.

pub(crate) type Overlay = crate::motion::Indicator;
