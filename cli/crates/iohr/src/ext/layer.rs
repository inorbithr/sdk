//! Reads the one program out of an extension's layer, a gzipped tar.
//!
//! Only the entrypoint is taken, and only as a regular file: links, devices and every
//! other entry are skipped, so nothing in a layer can write outside the install
//! directory. Sizes are bounded before they are read (no decompression bombs).

use std::io::Read;

use flate2::read::GzDecoder;

/// The largest program an extension may ship, uncompressed.
pub const MAX_PROGRAM: u64 = 512 * 1024 * 1024;
/// The most uncompressed bytes read from a layer before giving up.
const MAX_STREAM: u64 = MAX_PROGRAM + 64 * 1024 * 1024;
const BLOCK: usize = 512;
const MAX_META: u64 = 64 * 1024;

/// Why the program could not be taken from the layer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the extension's layer: {0}")]
pub struct LayerError(pub String);

/// The bytes of `entrypoint` in the gzipped tar `layer`.
///
/// # Errors
///
/// [`LayerError`] when the layer is not a gzipped tar, the entrypoint is missing or is
/// not a regular file, or a size bound is crossed.
pub fn program(layer: &[u8], entrypoint: &str) -> Result<Vec<u8>, LayerError> {
    let mut r = GzDecoder::new(layer).take(MAX_STREAM);
    let mut long_name: Option<String> = None;
    let mut pax_path: Option<String> = None;
    let mut pax_size: Option<u64> = None;
    loop {
        let mut header = [0u8; BLOCK];
        if !read_block(&mut r, &mut header)? || header.iter().all(|b| *b == 0) {
            return Err(LayerError(format!(
                "it holds no file {}",
                super::manifest::printable(entrypoint)
            )));
        }
        check_sum(&header)?;
        let size = pax_size
            .take()
            .map_or_else(|| octal(&header[124..136]), Ok)?;
        let kind = header[156];
        let name = long_name
            .take()
            .or_else(|| pax_path.take())
            .unwrap_or_else(|| header_name(&header));
        let padded = size.div_ceil(BLOCK as u64) * BLOCK as u64;
        match kind {
            b'x' | b'L' => {
                if size > MAX_META {
                    return Err(LayerError("a tar header is too large".into()));
                }
                let data = read_exact(&mut r, size, padded)?;
                if kind == b'L' {
                    long_name = Some(
                        String::from_utf8_lossy(&data)
                            .trim_end_matches('\0')
                            .to_owned(),
                    );
                } else {
                    let (p, s) = pax(&data)?;
                    pax_path = p;
                    pax_size = s;
                }
            }
            _ if normalise(&name) == entrypoint => {
                if !matches!(kind, b'0' | 0) {
                    return Err(LayerError(format!(
                        "{} is not a regular file",
                        super::manifest::printable(entrypoint)
                    )));
                }
                if size > MAX_PROGRAM {
                    return Err(LayerError(format!(
                        "the program is larger than {} MiB",
                        MAX_PROGRAM / 1024 / 1024
                    )));
                }
                return read_exact(&mut r, size, size);
            }
            _ => skip(&mut r, padded)?,
        }
    }
}

fn normalise(name: &str) -> &str {
    let mut n = name;
    while let Some(rest) = n.strip_prefix("./") {
        n = rest;
    }
    n
}

fn header_name(h: &[u8; BLOCK]) -> String {
    let field = |b: &[u8]| {
        let end = b.iter().position(|c| *c == 0).unwrap_or(b.len());
        String::from_utf8_lossy(&b[..end]).into_owned()
    };
    let name = field(&h[0..100]);
    if &h[257..262] == b"ustar" {
        let prefix = field(&h[345..500]);
        if !prefix.is_empty() {
            return format!("{prefix}/{name}");
        }
    }
    name
}

fn octal(field: &[u8]) -> Result<u64, LayerError> {
    if field.first().is_some_and(|b| b & 0x80 != 0) {
        return Err(LayerError(
            "a tar entry uses a size encoding iohr does not read".into(),
        ));
    }
    let text: String = field
        .iter()
        .take_while(|b| **b != 0)
        .map(|b| char::from(*b))
        .collect();
    let text = text.trim();
    if text.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(text, 8).map_err(|_| LayerError("a tar header has a bad size".into()))
}

fn check_sum(h: &[u8; BLOCK]) -> Result<(), LayerError> {
    let want = octal(&h[148..156])?;
    let got: u64 = h
        .iter()
        .enumerate()
        .map(|(i, b)| {
            if (148..156).contains(&i) {
                32
            } else {
                u64::from(*b)
            }
        })
        .sum();
    if want == got {
        Ok(())
    } else {
        Err(LayerError("a tar header's checksum is wrong".into()))
    }
}

/// The `path` and `size` records of a pax extended header.
fn pax(data: &[u8]) -> Result<(Option<String>, Option<u64>), LayerError> {
    let bad = || LayerError("a pax header cannot be read".into());
    let mut rest = data;
    let (mut path, mut size) = (None, None);
    while !rest.is_empty() {
        let space = rest.iter().position(|b| *b == b' ').ok_or_else(bad)?;
        let len: usize = std::str::from_utf8(&rest[..space])
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or_else(bad)?;
        if len <= space + 1 || len > rest.len() {
            return Err(bad());
        }
        let record = &rest[space + 1..len];
        let record = record.strip_suffix(b"\n").ok_or_else(bad)?;
        if let Some(eq) = record.iter().position(|b| *b == b'=') {
            let (k, v) = (&record[..eq], &record[eq + 1..]);
            match k {
                b"path" => path = Some(String::from_utf8_lossy(v).into_owned()),
                b"size" => {
                    size = Some(
                        std::str::from_utf8(v)
                            .ok()
                            .and_then(|s| s.parse().ok())
                            .ok_or_else(bad)?,
                    );
                }
                _ => {}
            }
        }
        rest = &rest[len..];
    }
    Ok((path, size))
}

fn read_block(r: &mut impl Read, buf: &mut [u8; BLOCK]) -> Result<bool, LayerError> {
    let mut filled = 0;
    while filled < BLOCK {
        let n = r.read(&mut buf[filled..]).map_err(io)?;
        if n == 0 {
            return if filled == 0 {
                Ok(false)
            } else {
                Err(LayerError("it ends inside a tar header".into()))
            };
        }
        filled += n;
    }
    Ok(true)
}

fn read_exact(r: &mut impl Read, size: u64, padded: u64) -> Result<Vec<u8>, LayerError> {
    let mut data = Vec::new();
    r.take(size).read_to_end(&mut data).map_err(io)?;
    if data.len() as u64 != size {
        return Err(LayerError("it ends inside a file".into()));
    }
    skip(r, padded - size)?;
    Ok(data)
}

fn skip(r: &mut impl Read, n: u64) -> Result<(), LayerError> {
    let copied = std::io::copy(&mut r.take(n), &mut std::io::sink()).map_err(io)?;
    if copied == n {
        Ok(())
    } else {
        Err(LayerError("it ends inside an entry".into()))
    }
}

#[allow(clippy::needless_pass_by_value, reason = "used with map_err")]
fn io(e: std::io::Error) -> LayerError {
    LayerError(format!("it is not a readable gzipped tar ({e})"))
}

/// A gzipped tar of `files` (name, type flag, contents), for tests and the fuzz seed.
#[doc(hidden)]
#[must_use]
pub fn tar_gz(files: &[(&str, u8, &[u8])]) -> Vec<u8> {
    use std::io::Write as _;
    let mut tar = Vec::new();
    for (name, kind, body) in files {
        let mut h = [0u8; BLOCK];
        h[..name.len().min(100)].copy_from_slice(&name.as_bytes()[..name.len().min(100)]);
        h[100..107].copy_from_slice(b"0000755");
        h[124..135].copy_from_slice(format!("{:011o}", body.len()).as_bytes());
        h[136..147].copy_from_slice(b"00000000000");
        h[156] = *kind;
        h[257..263].copy_from_slice(b"ustar\0");
        h[263..265].copy_from_slice(b"00");
        h[148..156].copy_from_slice(b"        ");
        let sum: u32 = h.iter().map(|b| u32::from(*b)).sum();
        h[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
        tar.extend_from_slice(&h);
        tar.extend_from_slice(body);
        tar.resize(tar.len().div_ceil(BLOCK) * BLOCK, 0);
    }
    tar.extend_from_slice(&[0u8; 2 * BLOCK]);
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let _ = gz.write_all(&tar);
    gz.finish().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{program, tar_gz};

    #[test]
    fn takes_the_entrypoint_and_nothing_else() {
        let layer = tar_gz(&[
            ("README", b'0', b"hello"),
            ("bin", b'5', b""),
            ("./bin/agent", b'0', b"\x7fELF program"),
        ]);
        assert_eq!(program(&layer, "bin/agent").unwrap(), b"\x7fELF program");
        assert!(program(&layer, "agent").is_err());
    }

    #[test]
    fn refuses_a_link_where_the_program_should_be() {
        let layer = tar_gz(&[("agent", b'2', b"")]);
        let e = program(&layer, "agent").unwrap_err();
        assert!(e.0.contains("not a regular file"), "{e}");
    }

    #[test]
    fn refuses_what_is_not_a_gzipped_tar() {
        assert!(program(b"plain bytes", "agent").is_err());
        let mut layer = tar_gz(&[("agent", b'0', b"x")]);
        layer.truncate(layer.len() / 2);
        assert!(program(&layer, "agent").is_err());
    }
}
