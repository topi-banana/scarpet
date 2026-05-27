use clap::Parser;
use scarpet_syntax::{
    lexer::Token,
    parser::{Builder, Code},
};

#[derive(Parser)]
struct Cli {}

fn main() {
    let _ = Cli::parse();
    let code = Box::new(Code::new())
        .push(Token::Ident("println"))
        .push(Token::OpenParen)
        .push(Token::CloseParen)
        .push(Token::SemiColon);

    println!("{:?}", code);
}
