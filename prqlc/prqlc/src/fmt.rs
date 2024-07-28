use prqlc_parser::{
    lexer::{lex_source, lr::Tokens},
    parser::{parse_lr_to_pr, pr},
};

pub fn fmt(source: &str) -> String {
    let lr = lex_source(source).unwrap();
    let pr = parse_lr_to_pr(0, lr.0.clone()).0.unwrap();
    fmt_of_ast(lr, &pr)
}

pub fn fmt_of_ast(tokens: Tokens, pr: &Vec<pr::Stmt>) -> String {
    let mut r = String::new();
    use prqlc_parser::lexer::lr::TokenKind::*;
    for token in tokens.0 {
        let t = token.kind;
        match t {
            Ident(ident) => r += &ident,
            Start => {}
            // Comment => r += &token.text,
            _ => todo!(),
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;
    use prqlc_parser::lexer::lex_source;

    use super::*;

    #[test]
    fn test_fmt() {
        assert_snapshot!(fmt("from artists"), @"fromartists");
    }
}
