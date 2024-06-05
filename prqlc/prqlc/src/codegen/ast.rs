use std::collections::HashSet;

use once_cell::sync::Lazy;
use prqlc_ast::expr::{
    BinOp, BinaryExpr, ExprKind, Ident, IndirectionKind, InterpolateItem, SwitchCase, UnOp,
    UnaryExpr,
};
use prqlc_parser::TokenVec;
use regex::Regex;

use super::{WriteOpt, WriteSource};
use crate::codegen::SeparatedExprs;
use crate::{ast::*, ir::pl::PlFold};

pub(crate) fn write_expr(expr: &Expr) -> String {
    expr.write(WriteOpt::new_width(u16::MAX)).unwrap()
}

fn write_within<T: WriteSource>(node: &T, parent: &ExprKind, mut opt: WriteOpt) -> Option<String> {
    let parent_strength = binding_strength(parent);
    opt.context_strength = opt.context_strength.max(parent_strength);

    // FIXME: this is extremely hacky. Our issue is that in:
    //
    //   from a.b  # comment
    //
    // ...we're writing both `from a.b` and `a.b`, so we need to know which of
    // these to write comments for. I'm sure there are better ways to do it.
    let enable_comments = opt.enable_comments;
    opt.enable_comments = false;
    let out = node.write(opt.clone());
    opt.enable_comments = enable_comments;
    out
}

impl WriteSource for Expr {
    fn write(&self, mut opt: WriteOpt) -> Option<String> {
        let mut r = String::new();

        // if let Some(span) = self.span {
        //     if let Some(comment) = find_comment_before(span, &opt.tokens) {
        //         r += &comment.to_string();
        //     }
        // }
        if let Some(alias) = &self.alias {
            r += opt.consume(alias)?;
            r += opt.consume(" = ")?;
            opt.unbound_expr = false;
        }

        if !needs_parenthesis(self, &opt) {
            r += &self.kind.write(opt.clone())?;
        } else {
            let value = self.kind.write_between("(", ")", opt.clone());

            if let Some(value) = value {
                r += &value;
            } else {
                r += &break_line_within_parenthesis(&self.kind, &mut opt)?;
            }
        };

        if opt.enable_comments {
            if let Some(span) = self.span {
                // TODO: change underlying function so we can remove this
                if opt.tokens.0.is_empty() {
                    return Some(r);
                }

                let comments = find_comments_after(span, &opt.tokens);

                // If the first item is a comment, it's an inline comment, and
                // so add two spaces
                if matches!(
                    comments.first(),
                    Some(Token {
                        kind: TokenKind::Comment(_),
                        ..
                    })
                ) {
                    r += "  ";
                }

                for c in comments {
                    match c.kind {
                        // TODO: these are defined here since the debug
                        // representations aren't quite right (NewLine is `new
                        // line` as is used in error messages). But we should
                        // probably move them onto the Struct.
                        TokenKind::Comment(s) => r += format!("#{}", s).as_str(),
                        TokenKind::NewLine => r += "\n",
                        _ => unreachable!(),
                    }
                }
            }
        }
        Some(r)
    }
}

fn needs_parenthesis(this: &Expr, opt: &WriteOpt) -> bool {
    if opt.unbound_expr && can_bind_left(&this.kind) {
        return true;
    }

    let binding_strength = binding_strength(&this.kind);
    if opt.context_strength > binding_strength {
        // parent has higher binding strength, which means it would "steal" operand of this expr
        // => parenthesis are needed
        return true;
    }

    if opt.context_strength < binding_strength {
        // parent has higher binding strength, which means it would "steal" operand of this expr
        // => parenthesis are needed
        return false;
    }

    // parent has equal binding strength, which means that now associativity of this expr counts
    // for example:
    //   this=(a + b), parent=(a + b) + c
    //   asoc of + is left
    //   this is the left operand of parent
    //   => assoc_matches=true => we don't need parenthesis

    //   this=(a + b), parent=c + (a + b)
    //   asoc of + is left
    //   this is the right operand of parent
    //   => assoc_matches=false => we need parenthesis
    let assoc_matches = match opt.binary_position {
        super::Position::Left => associativity(&this.kind) == super::Position::Left,
        super::Position::Right => associativity(&this.kind) == super::Position::Right,
        super::Position::Unspecified => false,
    };

    !assoc_matches
}

impl WriteSource for ExprKind {
    fn write(&self, mut opt: WriteOpt) -> Option<String> {
        use ExprKind::*;

        match &self {
            Ident(ident) => Some(write_ident_part(ident)),
            Indirection { base, field } => {
                let mut r = base.write(opt.clone())?;
                opt.consume_width(r.len() as u16)?;

                r += opt.consume(".")?;
                match field {
                    IndirectionKind::Name(n) => {
                        r += opt.consume(n)?;
                    }
                    IndirectionKind::Position(i) => {
                        r += &opt.consume(i.to_string())?;
                    }
                    IndirectionKind::Star => r += "*",
                }
                Some(r)
            }

            Pipeline(pipeline) => SeparatedExprs {
                inline: " | ",
                line_end: "",
                exprs: &pipeline.exprs,
            }
            .write_between("(", ")", opt),

            Tuple(fields) => SeparatedExprs {
                exprs: fields,
                inline: ", ",
                line_end: ",",
            }
            .write_between("{", "}", opt),

            Array(items) => SeparatedExprs {
                exprs: items,
                inline: ", ",
                line_end: ",",
            }
            .write_between("[", "]", opt),

            Range(range) => {
                let mut r = String::new();
                if let Some(start) = &range.start {
                    let start = write_within(start.as_ref(), self, opt.clone())?;
                    r += opt.consume(&start)?;
                }

                r += opt.consume("..")?;

                if let Some(end) = &range.end {
                    r += &write_within(end.as_ref(), self, opt)?;
                }
                Some(r)
            }
            Binary(BinaryExpr { op, left, right }) => {
                let mut r = String::new();

                let mut opt_left = opt.clone();
                opt_left.binary_position = super::Position::Left;
                let left = write_within(left.as_ref(), self, opt_left)?;
                r += opt.consume(&left)?;

                r += opt.consume(" ")?;
                r += opt.consume(&op.to_string())?;
                r += opt.consume(" ")?;

                let mut opt_right = opt;
                opt_right.binary_position = super::Position::Right;
                r += &write_within(right.as_ref(), self, opt_right)?;
                Some(r)
            }
            Unary(UnaryExpr { op, expr }) => {
                let mut r = String::new();

                r += opt.consume(&op.to_string())?;
                r += &write_within(expr.as_ref(), self, opt)?;
                Some(r)
            }
            FuncCall(func_call) => {
                let mut r = String::new();

                let name = write_within(func_call.name.as_ref(), self, opt.clone())?;
                r += opt.consume(&name)?;
                opt.unbound_expr = true;

                for (name, arg) in &func_call.named_args {
                    r += opt.consume(" ")?;

                    r += opt.consume(name)?;

                    r += opt.consume(":")?;

                    let arg = write_within(arg, self, opt.clone())?;
                    r += opt.consume(&arg)?;
                }
                for arg in &func_call.args {
                    r += opt.consume(" ")?;

                    let arg = write_within(arg, self, opt.clone())?;
                    r += opt.consume(&arg)?;
                }
                Some(r)
            }
            Func(c) => {
                let mut r = "func ".to_string();
                if !c.generic_type_params.is_empty() {
                    r += opt.consume("<")?;
                    for generic_param in &c.generic_type_params {
                        r += opt.consume(&write_ident_part(&generic_param.name))?;
                        r += opt.consume(": ")?;
                        r += &opt.consume(
                            SeparatedExprs {
                                exprs: &generic_param.domain,
                                inline: " | ",
                                line_end: "|",
                            }
                            .write(opt.clone())?,
                        )?;
                    }
                    r += opt.consume("> ")?;
                }

                for param in &c.params {
                    r += opt.consume(&write_ident_part(&param.name))?;
                    r += opt.consume(" ")?;
                    if let Some(ty) = &param.ty {
                        let ty = ty.write_between("<", ">", opt.clone())?;
                        r += opt.consume(&ty)?;
                        r += opt.consume(" ")?;
                    }
                }
                for param in &c.named_params {
                    r += opt.consume(&write_ident_part(&param.name))?;
                    r += opt.consume(":")?;
                    r += opt.consume(&param.default_value.as_ref().unwrap().write(opt.clone())?)?;
                    r += opt.consume(" ")?;
                }
                r += opt.consume("-> ")?;

                if let Some(ty) = &c.return_ty {
                    let ty = ty.write_between("<", ">", opt.clone())?;
                    r += opt.consume(&ty)?;
                    r += opt.consume(" ")?;
                }

                // try a single line
                if let Some(body) = c.body.write(opt.clone()) {
                    r += &body;
                } else {
                    r += &break_line_within_parenthesis(c.body.as_ref(), &mut opt)?;
                }

                Some(r)
            }
            SString(parts) => display_interpolation("s", parts, opt),
            FString(parts) => display_interpolation("f", parts, opt),
            Literal(literal) => opt.consume(literal.to_string()),
            Case(cases) => {
                let mut r = String::new();
                r += "case ";
                r += &SeparatedExprs {
                    exprs: cases,
                    inline: ", ",
                    line_end: ",",
                }
                .write_between("[", "]", opt)?;
                Some(r)
            }
            Param(id) => Some(format!("${id}")),
            Internal(operator_name) => Some(format!("internal {operator_name}")),
        }
    }
}

fn break_line_within_parenthesis<T: WriteSource>(expr: &T, opt: &mut WriteOpt) -> Option<String> {
    let mut r = "(\n".to_string();
    opt.indent += 1;
    r += &opt.write_indent();
    opt.reset_line()?;
    r += &expr.write(opt.clone())?;
    r += "\n";
    opt.indent -= 1;
    r += &opt.write_indent();
    r += ")";
    Some(r)
}

fn binding_strength(expr: &ExprKind) -> u8 {
    match expr {
        // For example, if it's an Ident, it's basically infinite — a simple
        // ident never needs parentheses around it.
        ExprKind::Ident(_) => 100,

        // Stronger than a range, since `-1..2` is `(-1)..2`
        // Stronger than binary op, since `-x == y` is `(-x) == y`
        // Stronger than a func call, since `exists !y` is `exists (!y)`
        ExprKind::Unary(..) => 20,

        ExprKind::Range(_) => 19,

        ExprKind::Binary(BinaryExpr { op, .. }) => match op {
            BinOp::Pow => 19,
            BinOp::Mul | BinOp::DivInt | BinOp::DivFloat | BinOp::Mod => 18,
            BinOp::Add | BinOp::Sub => 17,
            BinOp::Eq
            | BinOp::Ne
            | BinOp::Gt
            | BinOp::Lt
            | BinOp::Gte
            | BinOp::Lte
            | BinOp::RegexSearch => 16,
            BinOp::Coalesce => 15,
            BinOp::And => 14,
            BinOp::Or => 13,
        },

        // Weaker than a child assign, since `select x = 1`
        // Weaker than a binary operator, since `filter x == 1`
        ExprKind::FuncCall(_) => 10,
        // ExprKind::FuncCall(_) if !is_parent => 2,
        ExprKind::Func(_) => 7,

        // other nodes should not contain any inner exprs
        _ => 100,
    }
}

fn associativity(expr: &ExprKind) -> super::Position {
    match expr {
        ExprKind::Binary(BinaryExpr { op, .. }) => match op {
            BinOp::Pow => super::Position::Right,
            BinOp::Eq
            | BinOp::Ne
            | BinOp::Gt
            | BinOp::Lt
            | BinOp::Gte
            | BinOp::Lte
            | BinOp::RegexSearch => super::Position::Unspecified,
            _ => super::Position::Left,
        },

        _ => super::Position::Unspecified,
    }
}

/// True if this expression could be mistakenly bound with an expression on the left.
fn can_bind_left(expr: &ExprKind) -> bool {
    matches!(
        expr,
        ExprKind::Unary(UnaryExpr {
            op: UnOp::EqSelf | UnOp::Add | UnOp::Neg,
            ..
        })
    )
}

impl WriteSource for Ident {
    fn write(&self, mut opt: WriteOpt) -> Option<String> {
        let width = self.path.iter().map(|p| p.len() + 1).sum::<usize>() + self.name.len();
        opt.consume_width(width as u16)?;

        let mut r = String::new();
        for part in &self.path {
            r += &write_ident_part(part);
            r += ".";
        }
        r += &write_ident_part(&self.name);
        Some(r)
    }
}

pub static KEYWORDS: Lazy<HashSet<&str>> = Lazy::new(|| {
    HashSet::from_iter([
        "let", "into", "case", "prql", "type", "module", "internal", "func",
    ])
});

pub static VALID_PRQL_IDENT: Lazy<Regex> = Lazy::new(|| {
    // Pomsky expression (regex is to Pomsky what SQL is to PRQL):
    // ^ ('*' | [ascii_alpha '_$'] [ascii_alpha ascii_digit '_$']* ) $
    Regex::new(r"^(?:\*|[a-zA-Z_$][a-zA-Z0-9_$]*)$").unwrap()
});

pub fn write_ident_part(s: &str) -> String {
    if VALID_PRQL_IDENT.is_match(s) && !KEYWORDS.contains(s) {
        s.to_string()
    } else {
        format!("`{}`", s)
    }
}

// impl WriteSource for ModuleDef {
//     fn write(&self, mut opt: WriteOpt) -> Option<String> {
//         codegen::WriteSource::write(&pl.stmts, codegen::WriteOpt::default()).unwrap()
//     }}

// /// Find a comment before a span. If there's exactly one newline prior, then the
// /// comment is included here. Any further above are included with the prior token.
// fn find_comment_before(span: Span, tokens: &TokenVec) -> Option<TokenKind> {
//     // index of the span in the token vec
//     let index = tokens
//         .0
//         .iter()
//         .position(|t| t.span.start == span.start && t.span.end == span.end)?;
//     if index <= 1 {
//         return None;
//     }
//     let prior_token = &tokens.0[index - 1].kind;
//     let prior_2_token = &tokens.0[index - 2].kind;
//     if matches!(prior_token, TokenKind::NewLine) && matches!(prior_2_token, TokenKind::Comment(_)) {
//         Some(prior_2_token.clone())
//     } else {
//         None
//     }
// }

/// Find comments after a given span.
fn find_comments_after(span: Span, tokens: &TokenVec) -> Vec<Token> {
    // index of the span in the token vec
    let index = tokens
        .0
        .iter()
        // FIXME: why isn't this working?
        // .position(|t| t.1.start == span.start && t.1.end == span.end)
        .position(|t| t.span.end == span.end)
        .unwrap_or_else(|| panic!("{:?}, {:?}", &tokens, &span));

    let mut out = vec![];
    for token in tokens.0.iter().skip(index + 1) {
        match token.kind {
            TokenKind::NewLine | TokenKind::Comment(_) => out.push(token.clone()),
            _ => break,
        }
    }
    out
}

/// Add comments to the PL AST from the tokens.
fn add_comments_to_pl(tokens: &TokenVec, pl: &mut Vec<Stmt>) {
    // Iterate through tokens and when we find a comment or whitespace, add it
    // to an Expr in the PL

    let comments = tokens.0.iter().filter(|t| {
        matches!(
            t.kind,
            TokenKind::Comment(_) | TokenKind::NewLine | TokenKind::LineWrap(_)
        )
    });
}

/// Adds comments to the PL AST from the tokens.
struct AestheticTokensAdder {
    /// Tokens stored in reverse order, so final token is the first token
    pub aesthetic_tokens: Vec<Token>,
}

impl AestheticTokensAdder {
    pub fn new(mut aesthetic_tokens: Vec<Token>) -> Self {
        aesthetic_tokens.reverse();
        Self { aesthetic_tokens }
    }
}

use crate::Result;

impl PlFold for AestheticTokensAdder {
    fn fold_expr(&mut self, expr: Expr) -> Result<Expr> {
        let Some(next_token) = self.aesthetic_tokens.last() else {
            return Ok(expr);
        };

        let mut expr = expr;

        // If the comment comes before the expr, add it to the
        // previous_comments. This can only happen at the start of a statement;
        // otherwise we've already grabbed the comment and added it to a
        // previous expr.
        if next_token.span.start < expr.span.start {
            expr.comments_before
        }

        // expr.kind = match expr.kind {
        //     // these are values already
        //     ExprKind::Literal(l) => ExprKind::Literal(l),

        //     // these are values, iff their contents are values too
        //     ExprKind::Array(_) | ExprKind::Tuple(_) => self.fold_expr_kind(expr.kind)?,

        //     // functions are values
        //     ExprKind::Func(f) => ExprKind::Func(f),

        //     // ident are not values
        //     ExprKind::Ident(ident) => {
        //         // here we'd have to implement the whole name resolution, but for now,
        //         // let's do something simple

        //         // this is very crude, but for simple cases, it's enough
        //         let mut ident = ident;
        //         let mut base = self.context.clone();
        //         loop {
        //             let (first, remaining) = ident.pop_front();
        //             let res = lookup(base.as_ref(), &first).with_span(expr.span)?;

        //             if let Some(remaining) = remaining {
        //                 ident = remaining;
        //                 base = Some(res);
        //             } else {
        //                 return Ok(res);
        //             }
        //         }
        //     }

        //     // the beef happens here
        //     ExprKind::FuncCall(func_call) => {
        //         let func = self.fold_expr(*func_call.name)?;
        //         let mut func = func.try_cast(|x| x.into_func(), Some("func call"), "function")?;

        //         func.args.extend(func_call.args);

        //         if func.args.len() < func.params.len() {
        //             ExprKind::Func(func)
        //         } else {
        //             self.eval_function(*func, expr.span)?
        //         }
        //     }

        //     ExprKind::All { .. }
        //     | ExprKind::TransformCall(_)
        //     | ExprKind::SString(_)
        //     | ExprKind::FString(_)
        //     | ExprKind::Case(_)
        //     | ExprKind::RqOperator { .. }
        //     | ExprKind::Param(_)
        //     | ExprKind::Internal(_) => {
        //         return Err(Error::new_simple("not a value").with_span(expr.span))
        //     }
        // };
        // Ok(expr)
    }
}

impl WriteSource for Vec<Stmt> {
    fn write(&self, mut opt: WriteOpt) -> Option<String> {
        opt.reset_line()?;

        let mut r = String::new();
        for stmt in self {
            if !r.is_empty() {
                r += "\n";
            }

            r += &opt.write_indent();
            r += &stmt.write_or_expand(opt.clone());
        }
        Some(r)
    }
}

impl WriteSource for Stmt {
    fn write(&self, mut opt: WriteOpt) -> Option<String> {
        let mut r = String::new();

        for annotation in &self.annotations {
            r += "@";
            r += &annotation.expr.write(opt.clone())?;
            r += "\n";
            r += &opt.write_indent();
            opt.reset_line()?;
        }

        match &self.kind {
            StmtKind::QueryDef(query) => {
                r += "prql";
                if let Some(version) = &query.version {
                    r += &format!(r#" version:"{}""#, version);
                }
                for (key, value) in &query.other {
                    r += &format!(" {key}:{value}");
                }
                r += "\n";
            }
            StmtKind::VarDef(var_def) => match var_def.kind {
                _ if var_def.value.is_none() || var_def.ty.is_some() => {
                    let typ = if let Some(ty) = &var_def.ty {
                        format!("<{}> ", ty.write(opt.clone())?)
                    } else {
                        "".to_string()
                    };

                    r += opt.consume(&format!("let {} {}", var_def.name, typ))?;

                    if let Some(val) = &var_def.value {
                        r += opt.consume("= ")?;
                        r += &val.write(opt)?;
                    }
                    r += "\n";
                }

                VarDefKind::Let => {
                    r += opt.consume(&format!("let {} = ", var_def.name))?;

                    r += &var_def.value.as_ref().unwrap().write(opt)?;
                    r += "\n";
                }
                VarDefKind::Into | VarDefKind::Main => {
                    let val = var_def.value.as_ref().unwrap();
                    match &val.kind {
                        ExprKind::Pipeline(pipeline) => {
                            for expr in &pipeline.exprs {
                                r += &expr.write(opt.clone())?;
                                r += "\n";
                            }
                        }
                        _ => {
                            r += &val.write(opt)?;
                            r += "\n";
                        }
                    }

                    if var_def.kind == VarDefKind::Into {
                        r += &format!("into {}", var_def.name);
                        r += "\n";
                    }
                }
            },
            StmtKind::TypeDef(type_def) => {
                r += opt.consume(&format!("type {}", type_def.name))?;

                if let Some(ty) = &type_def.value {
                    r += opt.consume(" = ")?;
                    r += &ty.kind.write(opt)?;
                }
                r += "\n";
            }
            StmtKind::ModuleDef(module_def) => {
                r += &format!("module {} {{\n", module_def.name);
                opt.indent += 1;

                r += &module_def.stmts.write(opt.clone())?;

                opt.indent -= 1;
                r += &opt.write_indent();
                r += "}\n";
            }
            StmtKind::ImportDef(import_def) => {
                r += "import ";
                if let Some(alias) = &import_def.alias {
                    r += &write_ident_part(alias);
                    r += " = ";
                }
                r += &import_def.name.write(opt)?;
                r += "\n";
            }
        }
        Some(r)
    }
}

fn display_interpolation(prefix: &str, parts: &[InterpolateItem], opt: WriteOpt) -> Option<String> {
    let mut r = String::new();
    r += prefix;
    r += "\"";
    for part in parts {
        match &part {
            // We use double braces to escape braces
            InterpolateItem::String(s) => r += s.replace('{', "{{").replace('}', "}}").as_str(),
            InterpolateItem::Expr { expr, .. } => {
                r += "{";
                r += &expr.write(opt.clone())?;
                r += "}"
            }
        }
    }
    r += "\"";
    Some(r)
}

impl WriteSource for SwitchCase {
    fn write(&self, opt: WriteOpt) -> Option<String> {
        let mut r = String::new();
        r += &self.condition.write(opt.clone())?;
        r += " => ";
        r += &self.value.write(opt)?;
        Some(r)
    }
}

#[cfg(test)]
mod test {
    use insta::{assert_debug_snapshot, assert_snapshot};
    use prqlc_parser::lex_source;

    use super::*;

    #[track_caller]
    fn assert_is_formatted(input: &str) {
        let formatted = format_single_stmt(input);
        similar_asserts::assert_eq!(input.trim(), formatted.trim());
    }

    fn format_single_stmt(query: &str) -> String {
        use itertools::Itertools;
        let stmt = crate::prql_to_pl(query)
            .unwrap()
            .stmts
            .into_iter()
            .exactly_one()
            .unwrap();
        stmt.write(WriteOpt::default()).unwrap()
    }

    // #[test]
    // fn test_find_comment_before() {
    //     let tokens = lex_source(
    //         r#"
    //         # comment
    //         let a = 5
    //         "#,
    //     )
    //     .unwrap();
    //     let span = tokens
    //         .clone()
    //         .0
    //         .iter()
    //         .find(|t| t.kind == TokenKind::Keyword("let".to_string()))
    //         .unwrap()
    //         .span
    //         .clone();
    //     let comment = find_comment_before(span.into(), &tokens);
    //     assert_debug_snapshot!(comment, @r###"
    //     Some(
    //         Comment(
    //             " comment",
    //         ),
    //     )
    //     "###);
    // }

    #[test]
    fn test_find_comments_after() {
        let tokens = lex_source(
            r#"
            let a = 5 # on side
            # below
            # and another
            "#,
        )
        .unwrap();
        let span = tokens
            .clone()
            .0
            .iter()
            .find(|t| t.kind == TokenKind::Literal(Literal::Integer(5)))
            .unwrap()
            .span
            .clone();
        let comment = find_comments_after(span.into(), &tokens);
        assert_debug_snapshot!(comment, @r###"
        [
            23..32: Comment(" on side"),
            32..33: NewLine,
            45..52: Comment(" below"),
            52..53: NewLine,
            65..78: Comment(" and another"),
            78..79: NewLine,
        ]
        "###);
    }

    #[test]
    fn test_pipeline() {
        let short = Expr::new(ExprKind::Ident("short".to_string()));
        let long = Expr::new(ExprKind::Ident(
            "some_really_long_and_really_long_name".to_string(),
        ));

        let mut opt = WriteOpt {
            indent: 1,
            ..Default::default()
        };

        // short pipelines should be inlined
        let pipeline = Expr::new(ExprKind::Pipeline(Pipeline {
            exprs: vec![short.clone(), short.clone(), short.clone()],
        }));
        assert_snapshot!(pipeline.write(opt.clone()).unwrap(), @"(short | short | short)");

        // long pipelines should be indented
        let pipeline = Expr::new(ExprKind::Pipeline(Pipeline {
            exprs: vec![short.clone(), long.clone(), long, short.clone()],
        }));
        // colons are a workaround to avoid trimming
        assert_snapshot!(pipeline.write(opt.clone()).unwrap(), @r###"
        (
            short
            some_really_long_and_really_long_name
            some_really_long_and_really_long_name
            short
          )
        "###);

        // sometimes, there is just not enough space
        opt.rem_width = 4;
        opt.indent = 100;
        let pipeline = Expr::new(ExprKind::Pipeline(Pipeline { exprs: vec![short] }));
        assert!(pipeline.write(opt).is_none());
    }

    #[test]
    fn test_escaped_string() {
        assert_is_formatted(r#"filter name ~= "\\(I Can't Help\\) Falling""#);
    }

    #[test]
    fn test_double_braces() {
        assert_is_formatted(
            r#"let has_valid_title = s"regexp_contains(title, '([a-z0-9]*-){{2,}}')""#,
        );
    }

    #[test]
    fn test_unary() {
        assert_is_formatted(r#"sort {-duration}"#);

        assert_is_formatted(r#"select a = -b"#);
        assert_is_formatted(r#"join `project-bar.dataset.table` (==col_bax)"#);
    }

    #[test]
    fn test_binary() {
        assert_is_formatted(r#"let a = 5 * (4 + 3) ?? 5 / 2 // 2 == 1 and true"#);

        assert_is_formatted(r#"let a = 5 / 2 / 2"#);
        assert_is_formatted(r#"let a = 5 / (2 / 2)"#);

        // TODO: parsing for pow operator
        // assert_is_formatted(r#"let a = (5 ** 2) ** 2"#);
        // assert_is_formatted(r#"let a = 5 ** 2 ** 2"#);
    }

    #[test]
    fn test_func() {
        assert_is_formatted(r#"let a = func x y:false -> x and y"#);
    }

    #[test]
    fn test_simple() {
        assert_is_formatted(
            r#"
aggregate average_country_salary = (
  average salary
)"#,
        );
    }

    #[test]
    fn test_assign() {
        assert_is_formatted(
            r#"
group {title, country} (aggregate {
  average salary,
  average gross_salary,
  sum salary,
  sum gross_salary,
  average gross_cost,
  sum_gross_cost = sum gross_cost,
  ct = count salary,
})"#,
        );
    }
    #[test]
    fn test_range() {
        assert_is_formatted(
            r#"
let negative = -100..0
"#,
        );

        assert_is_formatted(
            r#"
let negative = -(100..0)
"#,
        );

        assert_is_formatted(
            r#"
let negative = -100..
"#,
        );

        assert_is_formatted(
            r#"
let negative = ..-100
"#,
        );
    }

    #[test]
    fn test_annotation() {
        assert_is_formatted(
            r#"
@deprecated
module hello {
}
"#,
        );
    }

    #[test]
    fn test_var_def() {
        assert_is_formatted(
            r#"
let a
"#,
        );

        assert_is_formatted(
            r#"
let a <int>
"#,
        );

        assert_is_formatted(
            r#"
let a = 5
"#,
        );

        assert_is_formatted(
            r#"
5
into a
"#,
        );
    }

    #[test]
    fn test_query_def() {
        assert_is_formatted(
            r#"
prql version:"^0.9" target:sql.sqlite
"#,
        );
    }
}
