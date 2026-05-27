use clap::Parser;
use scarpet_syntax::parser::Code;

#[derive(Parser)]
struct Cli {}

fn main() {
    let _ = Cli::parse();
    let code = Code::from_source("println();").expect("lex error");
    println!("{:?}", code.parse());
}
