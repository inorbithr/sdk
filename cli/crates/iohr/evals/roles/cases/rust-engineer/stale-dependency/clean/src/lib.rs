//! Relays HTTP/2 frames to the backend.

/// The largest frame the relay accepts, in bytes.
pub const MAX_FRAME: u32 = 16_384;

/// Whether `size` fits in one frame.
#[must_use]
pub fn fits(size: u32) -> bool {
    size <= MAX_FRAME
}

/// The h2 settings the relay offers its peers.
#[must_use]
pub fn server() -> h2::server::Builder {
    let mut b = h2::server::Builder::new();
    b.max_frame_size(MAX_FRAME);
    b
}
