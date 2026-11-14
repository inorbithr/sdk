//! Auto traits are part of the published API: losing one is a breaking change that
//! `cargo semver-checks` reports as `auto_trait_impl_removed`. This fails to compile
//! if a field makes `LoadOptions` (or the new sign-in credential) lose one.

use std::panic::{RefUnwindSafe, UnwindSafe};

fn all<T: Send + Sync + Unpin + UnwindSafe + RefUnwindSafe>() {}

#[test]
fn load_options_keeps_its_auto_traits() {
    all::<inorbithr::config::LoadOptions>();
}

#[cfg(feature = "login")]
#[test]
fn the_login_credential_has_the_auto_traits_of_a_provider() {
    fn shared<T: Send + Sync>() {}
    shared::<inorbithr::login::StoredSession>();
}
