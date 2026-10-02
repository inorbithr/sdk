use std::ffi::OsString;

use crate::error::Error;

/// Refuses a token or secret given as an argument (SR-24).
///
/// Arguments end up in shell history and in the process list other users can read.
/// The value is never echoed back.
///
/// # Errors
///
/// [`Error::Usage`] when an argument, or the value of a `key=value` argument, looks
/// like a JWT or a `Bearer` header.
pub fn refuse_secrets_in_args(args: &[OsString]) -> Result<(), Error> {
    let suspicious = args.iter().skip(1).any(|a| {
        let a = a.to_string_lossy();
        a.contains("Bearer ") || a.split(['=', ':', ' ']).any(looks_like_jwt)
    });
    if suspicious {
        return Err(Error::Usage(
            "a token was given as an argument, where shell history and the process list keep it. \
             Pipe it instead (`iohr login --with-token < token.txt`) or set IOHR_TOKEN, \
             then revoke this token if anyone else can see your history."
                .into(),
        ));
    }
    Ok(())
}

fn looks_like_jwt(s: &str) -> bool {
    s.len() > 30 && s.starts_with("eyJ") && s.matches('.').count() == 2
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::refuse_secrets_in_args;

    fn args(v: &[&str]) -> Vec<OsString> {
        std::iter::once("iohr")
            .chain(v.iter().copied())
            .map(OsString::from)
            .collect()
    }

    const JWT: &str = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJ4In0.c2lnbmF0dXJl";

    #[test]
    fn refuses_a_token_anywhere_in_argv() {
        for a in [
            vec!["login", JWT],
            vec!["api", "GET", "/v1/me", "-f", &format!("token={JWT}")],
            vec!["--header", "Authorization: Bearer x"],
        ] {
            let e = refuse_secrets_in_args(&args(&a)).unwrap_err();
            assert!(!e.to_string().contains(JWT));
        }
    }

    #[test]
    fn lets_ordinary_arguments_through() {
        assert!(
            refuse_secrets_in_args(&args(&["api", "GET", "/v1/radar/digests", "-f", "limit=5"]))
                .is_ok()
        );
        assert!(
            refuse_secrets_in_args(&args(&["token", "create", "--name", "eyJ-ish name"])).is_ok()
        );
    }
}
