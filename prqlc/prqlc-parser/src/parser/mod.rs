use chumsky::{prelude::*, Stream};

use crate::error::Error;
use crate::lexer::lr;
use crate::span::Span;

mod common;
mod expr;
mod interpolation;
pub(crate) mod perror;
pub mod pr;
pub(crate) mod stmt;
#[cfg(test)]
mod test;
mod types;

pub fn parse_lr_to_pr(
    source: &str,
    source_id: u16,
    lr: Vec<lr::Token>,
) -> (Option<Vec<pr::Stmt>>, Vec<Error>) {
    // We don't want comments in the AST (but we do intend to use them as part of
    // formatting)
    let stream = prepare_stream(lr.into_iter(), source, source_id);
    let (pr, parse_errors) = ::chumsky::Parser::parse_recovery(&stmt::source(), stream);

    let errors = parse_errors.into_iter().map(|e| e.into()).collect();
    log::debug!("parse errors: {errors:?}");

    (pr, errors)
}

/// Convert the output of the lexer into the input of the parser. Requires
/// supplying the original source code.
pub(crate) fn prepare_stream(
    tokens: impl Iterator<Item = lr::Token>,
    source: &str,
    source_id: u16,
) -> Stream<lr::TokenKind, Span, impl Iterator<Item = (lr::TokenKind, Span)> + Sized> {
    // We don't want comments in the AST (but we do intend to use them as part of
    // formatting)
    let semantic_tokens = tokens.filter(|token| {
        !matches!(
            token.kind,
            lr::TokenKind::Comment(_) | lr::TokenKind::LineWrap(_) | lr::TokenKind::DocComment(_)
        )
    });

    let tokens = semantic_tokens
        .into_iter()
        .map(move |token| (token.kind, Span::new(source_id, token.span)));
    let len = source.chars().count();
    let eoi = Span {
        start: len,
        end: len + 1,
        source_id,
    };
    Stream::from_iter(eoi, tokens)
}