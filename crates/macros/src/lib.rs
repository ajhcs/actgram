//! Declarative syntax for building and validating Actgram grammars.

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as Tokens};
use quote::quote;
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::{parenthesized, parse_macro_input, Ident, LitInt, LitStr, Result, Token};

mod keyword {
    syn::custom_keyword!(start);
}

/// Build a grammar using the actgram-core builder. Bare identifiers and
/// strings name actions. A rule reference has an @ prefix. A repeat's
/// maximum can be * for an unbounded repeat.
///
///     use actgram_macros::grammar;
///
///     let g = grammar! {
///         start main;
///         main = seq(push, repeat(@read, 0, 3), "check queue");
///         read = alt(2 => peek, 1 => pop);
///     }.unwrap();
///     assert_eq!(g.analysis()[g.root()].minimum_actions(), Some(2));
///
/// The calling crate declares a dependency on actgram-core.
#[proc_macro]
pub fn grammar(input: TokenStream) -> TokenStream {
    let Syntax { start, rules } = parse_macro_input!(input as Syntax);
    let builder = Ident::new("__actgram_builder", Span::mixed_site());
    let definitions = rules.into_iter().map(|(name, body)| {
        let name = name.unraw().to_string();
        let expression = body.emit(&builder);
        quote! {
            let definition = #expression;
            #builder.rule(#name, definition)?;
        }
    });
    let start = start.unraw().to_string();
    quote! {
        (|| {
            let mut #builder = ::actgram_core::Builder::new();
            #(#definitions)*
            let root = #builder.symbol(#start);
            #builder.build(root)
        })()
    }
    .into()
}

struct Syntax {
    start: Ident,
    rules: Vec<(Ident, Expr)>,
}

impl Parse for Syntax {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        input.parse::<keyword::start>()?;
        let start = input.parse()?;
        input.parse::<Token![;]>()?;
        let mut rules = Vec::new();
        while !input.is_empty() {
            let name = input.parse()?;
            input.parse::<Token![=]>()?;
            let body = input.parse()?;
            input.parse::<Token![;]>()?;
            rules.push((name, body));
        }
        Ok(Self { start, rules })
    }
}

enum Expr {
    Action(String),
    Symbol(String),
    Seq(Vec<Expr>),
    Alt(Vec<Arm>),
    Repeat(Box<Expr>, u32, Option<u32>),
}

struct Arm {
    weight: u32,
    child: Expr,
}

impl Parse for Arm {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let weight = input.parse::<LitInt>()?.base10_parse()?;
        input.parse::<Token![=>]>()?;
        let child = input.parse()?;
        Ok(Self { weight, child })
    }
}

impl Parse for Expr {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        if input.peek(Token![@]) {
            input.parse::<Token![@]>()?;
            return Ok(Self::Symbol(input.parse::<Ident>()?.unraw().to_string()));
        }
        if input.peek(LitStr) {
            return Ok(Self::Action(input.parse::<LitStr>()?.value()));
        }
        let name: Ident = input.parse()?;
        if !input.peek(syn::token::Paren) {
            return Ok(Self::Action(name.unraw().to_string()));
        }
        let arguments;
        parenthesized!(arguments in input);
        match name.to_string().as_str() {
            "seq" => Ok(Self::Seq(
                arguments
                    .parse_terminated(Self::parse, Token![,])?
                    .into_iter()
                    .collect(),
            )),
            "alt" => Ok(Self::Alt(
                arguments
                    .parse_terminated(Arm::parse, Token![,])?
                    .into_iter()
                    .collect(),
            )),
            "repeat" => {
                let child = Box::new(arguments.parse()?);
                arguments.parse::<Token![,]>()?;
                let min = arguments.parse::<LitInt>()?.base10_parse()?;
                arguments.parse::<Token![,]>()?;
                let max = if arguments.peek(Token![*]) {
                    arguments.parse::<Token![*]>()?;
                    None
                } else {
                    Some(arguments.parse::<LitInt>()?.base10_parse()?)
                };
                if arguments.peek(Token![,]) {
                    arguments.parse::<Token![,]>()?;
                }
                if !arguments.is_empty() {
                    return Err(arguments.error("expected end of repeat arguments"));
                }
                Ok(Self::Repeat(child, min, max))
            }
            _ => Err(syn::Error::new(name.span(), "expected seq, alt, or repeat")),
        }
    }
}

impl Expr {
    fn emit(&self, builder: &Ident) -> Tokens {
        match self {
            Self::Action(name) => quote!(#builder.action(#name)),
            Self::Symbol(name) => quote!(#builder.symbol(#name)),
            Self::Seq(children) => {
                let children = children.iter().map(|child| child.emit(builder));
                quote!({ let children = [#(#children),*]; #builder.seq(children) })
            }
            Self::Alt(arms) => {
                let arms = arms.iter().map(|Arm { weight, child }| {
                    let child = child.emit(builder);
                    quote!((#weight, #child))
                });
                quote!({ let arms = [#(#arms),*]; #builder.alt(arms) })
            }
            Self::Repeat(child, min, max) => {
                let child = child.emit(builder);
                let max = match max {
                    Some(max) => quote!(::core::option::Option::Some(#max)),
                    None => quote!(::core::option::Option::None),
                };
                quote!({ let body = #child; #builder.repeat(body, #min, #max) })
            }
        }
    }
}
