fn hello() -> String {
    "hello from @@NAME@@".to_owned()
}

fn main() {
    println!("{}", hello());
}

#[cfg(test)]
mod tests {
    use super::hello;

    #[test]
    fn says_hello() {
        assert_eq!(hello(), "hello from @@NAME@@");
    }
}
