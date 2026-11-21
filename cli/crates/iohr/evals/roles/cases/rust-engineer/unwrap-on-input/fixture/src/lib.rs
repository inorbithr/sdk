//! Parses the `host:port` addresses users type into the connect box.

/// A host and a port.
#[derive(Debug, PartialEq, Eq)]
pub struct Addr {
    pub host: String,
    pub port: u16,
}

/// Parses `host:port`, as typed by a user.
pub fn parse(input: &str) -> Addr {
    let (host, port) = input.trim().rsplit_once(':').unwrap();
    Addr {
        host: host.to_owned(),
        port: port.parse().unwrap(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_host_and_port() {
        assert_eq!(parse("db.local:5432"), Addr { host: "db.local".into(), port: 5432 });
    }
}
