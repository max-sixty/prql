use prqlc_parser::{
    lexer,
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
            Start => {}
            Literal(l) => r += &l.to_string(),
            // Ident(ident) => r += &ident,
            Control(c) => r += &c.to_string(),
            // Comment => r += &token.text,
            _ => {
                dbg!(&t);
                todo!()
            }
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;

    use super::*;

    #[test]
    fn test_fmt() {
        assert_snapshot!(fmt("[2, 3, 4]"), @"[2,3,4]");
    }
}
