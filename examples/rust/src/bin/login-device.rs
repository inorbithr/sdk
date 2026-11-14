//! A person signs in with the device flow (RFC 8628) and the SDK keeps the session: the
//! `login` feature. Show the user code and the link, never the device code; once the person
//! approves, the session is saved and a client calls the API as that person, refreshing
//! the short access token with the rotating refresh token when it runs out.
//!
//! `INORBIT_BASE_URL` is the sign-in service here; unset, it is `https://auth.inorbit.hr`.
//! A program that wants the person's saved `iohr login` instead builds a client with
//! `Client::builder().load()`, where the `login` source of the credential chain does this.
//!
//! ```sh
//! cargo run --bin login-device
//! ```

use std::sync::Arc;

use inorbithr::login::{
    DEFAULT_CLIENT_ID, DEFAULT_ISSUER, EntryKey, Login, MemoryStore, Provider, Session,
    StoredSession,
};
use inorbithr::public::Surface as _;
use inorbithr::{Client, Public};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let issuer = std::env::var("INORBIT_BASE_URL").unwrap_or_else(|_| DEFAULT_ISSUER.to_owned());
    let provider = Provider::discover(&issuer, DEFAULT_CLIENT_ID).await?;

    let device = Login::device(&provider).await?;
    println!(
        "open {} and enter {}",
        device.verification_uri(),
        device.user_code()
    );
    let granted = device.finish().await?; // polls at the service's interval

    // A real program keeps the session where the person chose: `store_for` opens the
    // keyring or the owner-only files. This example keeps it in memory.
    let account = granted.account().unwrap_or("acc_1").to_owned();
    let key = EntryKey::new("example".parse()?, &account)?;
    let session = Session::save(granted, Arc::new(MemoryStore::default()), key).await?;

    let mut builder = Client::<Public>::builder().token_provider(StoredSession::new(session));
    if let Ok(base) = std::env::var("INORBIT_BASE_URL") {
        builder = builder.base_url(base);
    }
    let me = builder.build()?.me().await?;
    println!("signed in as {}, a {}", me.value.subject, me.value.kind);
    Ok(())
}
