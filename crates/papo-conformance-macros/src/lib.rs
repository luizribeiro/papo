//! Attribute macros for the papo conformance suite.

use proc_macro::TokenStream;
use syn::{
    Ident, LitStr, Result, Token, bracketed, parenthesized, parse::Parse, parse::ParseStream,
    parse_macro_input,
};

#[derive(Default)]
struct ScenarioArgs {
    caps: Option<Vec<LitStr>>,
    isupport: Option<Vec<LitStr>>,
}

impl Parse for ScenarioArgs {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        if input.is_empty() {
            return Ok(Self::default());
        }
        let keyword: Ident = input.parse()?;
        if keyword != "requires" {
            return Err(syn::Error::new(keyword.span(), "expected `requires(...)`"));
        }
        let content;
        parenthesized!(content in input);
        let mut args = Self::default();
        while !content.is_empty() {
            let name: Ident = content.parse()?;
            content.parse::<Token![=]>()?;
            let destination = if name == "caps" {
                &mut args.caps
            } else if name == "isupport" {
                &mut args.isupport
            } else {
                return Err(syn::Error::new(
                    name.span(),
                    "expected `caps` or `isupport`",
                ));
            };
            if destination.is_some() {
                return Err(syn::Error::new(name.span(), "requirement specified twice"));
            }
            *destination = Some(parse_strings(&content)?);
            if !content.is_empty() {
                content.parse::<Token![,]>()?;
            }
        }
        if !input.is_empty() {
            return Err(input.error("unexpected scenario arguments"));
        }
        Ok(args)
    }
}

fn parse_strings(input: ParseStream<'_>) -> Result<Vec<LitStr>> {
    let content;
    bracketed!(content in input);
    content
        .parse_terminated(syn::parse::ParseBuffer::parse, Token![,])
        .map(|values| values.into_iter().collect())
}

/// Validates the requirement syntax on a conformance scenario.
#[proc_macro_attribute]
pub fn scenario(args: TokenStream, input: TokenStream) -> TokenStream {
    let _: ScenarioArgs = parse_macro_input!(args as ScenarioArgs);
    input
}

#[cfg(test)]
mod tests {
    use super::ScenarioArgs;

    #[test]
    fn parses_optional_capability_and_isupport_requirements() {
        let empty = syn::parse_str::<ScenarioArgs>("").unwrap();
        assert!(empty.caps.is_none());
        assert!(empty.isupport.is_none());

        let args = syn::parse_str::<ScenarioArgs>(
            r#"requires(caps = ["echo-message", "sasl=SCRAM-SHA-256"], isupport = ["MONITOR"])"#,
        )
        .unwrap();
        assert_eq!(args.caps.unwrap()[1].value(), "sasl=SCRAM-SHA-256");
        assert_eq!(args.isupport.unwrap()[0].value(), "MONITOR");
    }

    #[test]
    fn rejects_unknown_and_duplicate_requirements() {
        for input in [
            r#"requires(tokens = ["WHOX"])"#,
            "other(caps = [])",
            "requires(caps = [], caps = [])",
        ] {
            assert!(syn::parse_str::<ScenarioArgs>(input).is_err());
        }
    }
}
