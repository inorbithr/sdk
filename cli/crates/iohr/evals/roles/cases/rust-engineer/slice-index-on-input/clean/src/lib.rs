//! Reads length-prefixed frames from bytes that came off the network.

/// Returns the first frame of `buf`: a big-endian u16 length, then that many bytes. `None`
/// when `buf` is shorter than its header or than the length it claims.
#[must_use]
pub fn first_frame(buf: &[u8]) -> Option<&[u8]> {
    let (head, rest) = buf.split_first_chunk::<2>()?;
    rest.get(..usize::from(u16::from_be_bytes(*head)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_frame() {
        assert_eq!(first_frame(&[0, 3, b'a', b'b', b'c', 9]), Some(&b"abc"[..]));
    }

    #[test]
    fn short_input_is_none() {
        assert_eq!(first_frame(&[0]), None);
        assert_eq!(first_frame(&[0, 9, b'a']), None);
    }
}
