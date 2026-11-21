//! Reads length-prefixed frames from bytes that came off the network.

/// Returns the first frame of `buf`: a big-endian u16 length, then that many bytes.
#[must_use]
pub fn first_frame(buf: &[u8]) -> &[u8] {
    let len = usize::from(u16::from_be_bytes([buf[0], buf[1]]));
    &buf[2..2 + len]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_frame() {
        assert_eq!(first_frame(&[0, 3, b'a', b'b', b'c', 9]), b"abc");
    }
}
