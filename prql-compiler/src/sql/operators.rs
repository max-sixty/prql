use std::collections::HashMap;
use std::iter::zip;
use std::path::PathBuf;

use anyhow::Result;
use itertools::Itertools;
use once_cell::sync::Lazy;

use super::gen_expr::{translate_operand, ExprOrSource};
use super::{Context, Dialect};

use crate::ast::{pl, rq};
use crate::error::WithErrorInfo;
use crate::semantic;
use crate::Error;

static STD: Lazy<semantic::Module> = Lazy::new(load_std_sql);

fn load_std_sql() -> semantic::Module {
    let std_lib = crate::SourceTree::new([(
        PathBuf::from("std.prql"),
        include_str!("./std.sql.prql").to_string(),
    )]);
    let ast = crate::parser::parse(&std_lib).unwrap();

    let options = semantic::ResolverOptions {
        allow_module_decls: true,
    };

    let context = semantic::resolve(ast, options).unwrap();
    context.root_mod
}

pub(super) fn translate_operator_expr(expr: rq::Expr, ctx: &mut Context) -> Result<ExprOrSource> {
    let (name, args) = expr.kind.into_operator().unwrap();

    let (text, binding_strength) = translate_operator(name, args, ctx).with_span(expr.span)?;

    Ok(ExprOrSource::Source {
        text,
        binding_strength,
    })
}

pub(super) fn translate_operator(
    name: String,
    args: Vec<rq::Expr>,
    ctx: &mut Context,
) -> Result<(String, i32)> {
    let (func_def, binding_strength) = find_operator_impl(&name, ctx.dialect_enum).unwrap();
    let parent_binding_strength = binding_strength.unwrap_or(100);

    let params = func_def
        .named_params
        .iter()
        .chain(func_def.params.iter())
        .map(|x| x.name.split('.').last().unwrap_or(x.name.as_str()));

    let args: HashMap<&str, _> = zip(params, args.into_iter()).collect();

    // body can only be an s-string
    let body = match &func_def.body.kind {
        pl::ExprKind::Literal(pl::Literal::Null) => {
            return Err(Error::new_simple(format!(
                "operator {} is not supported for dialect {}",
                name, ctx.dialect_enum
            ))
            .into())
        }
        pl::ExprKind::SString(items) => items,
        _ => panic!("Bad RQ operator implementation. Expected s-string or null"),
    };

    let mut text = String::new();

    for item in body {
        match item {
            pl::InterpolateItem::Expr { expr, format } => {
                // s-string exprs can only contain idents
                let ident = expr.kind.as_ident();
                let ident = ident.as_ref().unwrap();

                // lookup args
                let arg = args.get(ident.name.as_str()).unwrap().clone();

                // binding strength
                let required_strength = format
                    .as_ref()
                    .and_then(|f| f.parse::<i32>().ok())
                    .unwrap_or(parent_binding_strength);

                // translate args
                let arg = translate_operand(arg, required_strength, false, ctx)?;

                text += &arg.into_source();
            }
            pl::InterpolateItem::String(s) => {
                text += s;
            }
        }
    }

    Ok((text, parent_binding_strength))
}

fn find_operator_impl(operator_name: &str, dialect: Dialect) -> Option<(&pl::Func, Option<i32>)> {
    let operator_name = operator_name.strip_prefix("std.").unwrap();

    let operator_name = pl::Ident::from_name(operator_name);

    let dialect_module = STD.get(&pl::Ident::from_name(dialect.to_string()));

    let mut func_def = None;

    if let Some(dialect_module) = dialect_module {
        let module = dialect_module.kind.as_module().unwrap();
        func_def = module.get(&operator_name);
    }

    if func_def.is_none() {
        func_def = STD.get(&operator_name);
    }

    let decl = func_def?;

    let func_def = decl.kind.as_expr().unwrap();
    let func_def = func_def.kind.as_func().unwrap();

    let binding_strength = decl
        .clone()
        .annotations
        .into_iter()
        .exactly_one()
        .ok()
        .and_then(|x| x.tuple_items().ok())
        .and_then(|items| items.into_iter().find(|bs| bs.0 == "binding_strength"))
        .and_then(|tuple| tuple.1.into_literal().ok())
        .and_then(|literal| literal.into_integer().ok())
        .map(|int| int as i32);

    Some((func_def.as_ref(), binding_strength))
}
