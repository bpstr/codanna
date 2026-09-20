#![allow(dead_code)]

struct Client;
struct Token;

impl Client {
    fn make_token() -> Token { Token }
    fn consume(&self) {}
}

impl Token {
    fn consume(&self) {}
}

fn inferred() {
    let value = Client::make_token();
    value.consume();
}

fn annotated() {
    let value: Token = Client::make_token();
    value.consume();
}

fn main() {}
