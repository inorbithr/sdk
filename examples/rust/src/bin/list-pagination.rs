//! A paged list. `all_list_monitors` yields every item and follows the next-page token for
//! you; the second half reads one page by hand with `page_size` and `page_token`.
//!
//! ```sh
//! INORBIT_ORG_ID=org_... INORBIT_TOKEN=... cargo run --bin list-pagination
//! ```

use inorbithr::public::{ConnectionsListMonitorsParams, Surface as _};
use inorbithr::{Client, Error};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let client: Client = Client::from_env()?;
    let connections = client.connections();
    let org = std::env::var("INORBIT_ORG_ID").unwrap_or_default();

    // Every item, page after page; the walk ends at the last page or at the first error.
    let params = ConnectionsListMonitorsParams {
        page_size: Some(2),
        ..Default::default()
    };
    let mut monitors = connections.all_list_monitors(&org, &params);
    let mut n = 0;
    while let Some(monitor) = monitors.next().await {
        let monitor = monitor?;
        n += 1;
        println!("{} {}", monitor.monitor_id, monitor.name);
    }
    println!("{n} monitors");

    // One page by hand: the next page token comes back with the page.
    let one = ConnectionsListMonitorsParams {
        page_size: Some(1),
        ..Default::default()
    };
    let page = connections.list_monitors(&org, &one).await?.value;
    println!(
        "one page: {} monitor, more after {:?}",
        page.monitors.len(),
        page.next_page_token
    );
    Ok(())
}
