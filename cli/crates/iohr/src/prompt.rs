//! Asking the person at the keyboard: a secret without echo, a confirmation, and the
//! line that says what `iohr` is waiting for. Everything goes to the terminal or
//! stderr, never to stdout, so pipes get only data.

use std::io::{self, BufRead as _, IsTerminal as _, Read as _, Write as _};
use std::path::Path;

use iohr_auth::Redacted;
use zeroize::Zeroizing;

use crate::error::Error;

/// The largest secret read from a file or standard input.
const MAX_SECRET: u64 = 64 * 1024;

/// Whether a person is at a terminal to answer a question.
pub(crate) fn interactive() -> bool {
    io::stderr().is_terminal()
}

/// Asks for a secret without echoing it. The answer is wrapped as it is read.
pub(crate) fn secret(label: &str) -> Result<Redacted<String>, Error> {
    if !interactive() {
        return Err(Error::Usage(format!(
            "{label} is needed and there is no terminal to ask on: pass --secret-file FIELD=PATH \
             or --secret-stdin FIELD"
        )));
    }
    let answer = Redacted::new(
        rpassword::prompt_password(format!("{label} (not shown): "))
            .map_err(|e| Error::Failed(format!("cannot read {label} from the terminal: {e}")))?,
    );
    nonempty(answer, label)
}

/// Reads a secret from a file, at most 64 KiB; surrounding white space is dropped.
pub(crate) fn secret_from_file(label: &str, path: &Path) -> Result<Redacted<String>, Error> {
    let file = std::fs::File::open(path)
        .map_err(|e| Error::Usage(format!("cannot read {label} from {}: {e}", path.display())))?;
    read_secret(file, label, &format!("{}", path.display()))
}

/// Reads a secret from standard input, at most 64 KiB.
pub(crate) fn secret_from_stdin(label: &str) -> Result<Redacted<String>, Error> {
    let stdin = io::stdin();
    if stdin.is_terminal() {
        return Err(Error::Usage(format!(
            "--secret-stdin reads {label} from standard input: pipe it in, or leave the flag out \
             to be asked for it"
        )));
    }
    read_secret(stdin.lock(), label, "standard input")
}

fn read_secret(r: impl io::Read, label: &str, from: &str) -> Result<Redacted<String>, Error> {
    let mut text = Zeroizing::new(String::new());
    r.take(MAX_SECRET + 1)
        .read_to_string(&mut text)
        .map_err(|e| Error::Usage(format!("cannot read {label} from {from}: {e}")))?;
    if text.len() as u64 > MAX_SECRET {
        return Err(Error::Usage(format!(
            "{label} from {from} is longer than 64 KiB"
        )));
    }
    nonempty(Redacted::new(text.trim().to_owned()), label)
}

fn nonempty(s: Redacted<String>, label: &str) -> Result<Redacted<String>, Error> {
    if s.expose().trim().is_empty() {
        Err(Error::Usage(format!("{label} was empty")))
    } else if s.expose().trim().len() == s.expose().len() {
        Ok(s)
    } else {
        Ok(Redacted::new(s.expose().trim().to_owned()))
    }
}

/// Asks for a value that is not secret, shown as it is typed.
pub(crate) fn line(question: &str) -> Result<String, Error> {
    let mut err = io::stderr().lock();
    let _ = write!(err, "{question}: ");
    let _ = err.flush();
    let mut answer = String::new();
    io::stdin()
        .lock()
        .read_line(&mut answer)
        .map_err(|e| Error::Failed(format!("cannot read the answer: {e}")))?;
    Ok(answer.trim().to_owned())
}

/// Asks the person to type `expected` to go ahead. Without a terminal, `refusal`.
pub(crate) fn confirm(question: &str, expected: &str, refusal: &str) -> Result<(), Error> {
    if !(interactive() && io::stdin().is_terminal()) {
        return Err(Error::Usage(refusal.to_owned()));
    }
    if line(question)? == expected {
        Ok(())
    } else {
        Err(Error::Usage("not confirmed; nothing was changed".into()))
    }
}

/// One line on stderr that says what is being waited for, redrawn in place on a
/// terminal; nothing at all otherwise.
#[derive(Debug)]
pub(crate) struct Waiting {
    on: bool,
    frame: usize,
}

impl Waiting {
    pub(crate) fn new(json: bool) -> Self {
        Self {
            on: !json && interactive(),
            frame: 0,
        }
    }

    pub(crate) fn tick(&mut self, text: &str) {
        const FRAMES: [char; 4] = ['|', '/', '-', '\\'];
        if !self.on {
            return;
        }
        let c = FRAMES[self.frame % FRAMES.len()];
        self.frame += 1;
        let mut err = io::stderr().lock();
        let _ = write!(err, "\r{c} {text}\x1b[K");
        let _ = err.flush();
    }

    pub(crate) fn done(&mut self) {
        if self.on {
            let mut err = io::stderr().lock();
            let _ = write!(err, "\r\x1b[K");
            let _ = err.flush();
            self.on = false;
        }
    }
}

impl Drop for Waiting {
    fn drop(&mut self) {
        self.done();
    }
}

#[cfg(test)]
mod tests {
    use super::read_secret;

    #[test]
    fn a_secret_is_trimmed_bounded_and_never_in_an_error() {
        let s = read_secret(&b"  sk-live-123\n"[..], "api_key", "a file").unwrap();
        assert_eq!(s.expose(), "sk-live-123");
        assert_eq!(format!("{s:?}"), "<redacted>");
        let e = read_secret(&b"\n \n"[..], "api_key", "a file").unwrap_err();
        assert!(e.to_string().contains("empty"));
        let big = vec![b'a'; 64 * 1024 + 1];
        let e = read_secret(&big[..], "api_key", "a file").unwrap_err();
        assert!(!e.to_string().contains("aaaa"));
    }
}
