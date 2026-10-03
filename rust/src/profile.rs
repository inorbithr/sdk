/// One way of calling the API: a name, and the environment variables a client built
/// with [`Client::from_env`](crate::Client::from_env) reads.
///
/// A generated SDK (`iohr sdk generate`) defines one zero-sized type per profile it was
/// generated for, and the operations that profile may call are implemented for that
/// type only, so a call the profile's credential cannot make does not compile.
/// [`Public`] is the profile of the surface this crate ships with: every operation the
/// public API offers, read from the bare `INORBIT_*` variables.
pub trait Profile: Send + Sync + 'static {
    /// The profile's name, as `iohr profile list` shows it (`personal`, `acme-ci`).
    const NAME: &'static str;
}

/// The profile of the public surface: the operations any credential may call, with
/// credentials read from `INORBIT_KEY_ID`, `INORBIT_KEY_SECRET` and `INORBIT_SCOPES`,
/// or `INORBIT_TOKEN`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Public;

impl Profile for Public {
    const NAME: &'static str = "public";
}

/// The prefix of the variables a profile reads: `INORBIT_` for the public profile,
/// `INORBIT_<NAME>_` otherwise, with the name in upper case and `-` as `_`.
pub(crate) fn env_prefix(name: &str) -> String {
    if name == Public::NAME {
        return "INORBIT_".to_owned();
    }
    let upper: String = name
        .chars()
        .map(|c| match c {
            '-' => '_',
            c => c.to_ascii_uppercase(),
        })
        .collect();
    format!("INORBIT_{upper}_")
}

#[cfg(test)]
mod tests {
    use super::env_prefix;

    #[test]
    fn the_public_profile_reads_the_bare_names_and_others_their_own() {
        assert_eq!(env_prefix("public"), "INORBIT_");
        assert_eq!(env_prefix("personal"), "INORBIT_PERSONAL_");
        assert_eq!(env_prefix("acme-ci"), "INORBIT_ACME_CI_");
    }
}
