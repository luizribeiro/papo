//! Attribute macros for the papo conformance suite.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{
    Attribute, Ident, ItemFn, LitStr, Result, Token, bracketed, parenthesized, parse::Parse,
    parse::ParseStream, parse_macro_input,
};

#[derive(Default)]
struct ScenarioArgs {
    caps: Option<Vec<LitStr>>,
    isupport: Option<Vec<LitStr>>,
    covers: Option<Vec<LitStr>>,
}

impl Parse for ScenarioArgs {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        if input.is_empty() {
            return Ok(Self::default());
        }
        let mut args = Self::default();
        while !input.is_empty() {
            let keyword: Ident = input.parse()?;
            if keyword == "requires" {
                let content;
                parenthesized!(content in input);
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
            } else if keyword == "covers" {
                input.parse::<Token![=]>()?;
                if args.covers.is_some() {
                    return Err(syn::Error::new(keyword.span(), "coverage specified twice"));
                }
                args.covers = Some(parse_strings(input)?);
            } else {
                return Err(syn::Error::new(
                    keyword.span(),
                    "expected `requires(...)` or `covers = [...]`",
                ));
            }
            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
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

fn doc_line(attribute: &Attribute) -> Option<&LitStr> {
    let syn::Meta::NameValue(meta) = &attribute.meta else {
        return None;
    };
    let syn::Expr::Lit(expression) = &meta.value else {
        return None;
    };
    let syn::Lit::Str(line) = &expression.lit else {
        return None;
    };
    meta.path.is_ident("doc").then_some(line)
}

fn description(function: &ItemFn) -> Result<LitStr> {
    let lines = function
        .attrs
        .iter()
        .filter_map(doc_line)
        .collect::<Vec<_>>();
    let text = lines
        .iter()
        .map(|line| line.value().trim().to_owned())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() {
        return Err(syn::Error::new_spanned(
            &function.sig.ident,
            "a scenario requires a doc comment",
        ));
    }
    Ok(LitStr::new(&text, lines[0].span()))
}

fn expand(args: ScenarioArgs, function: &ItemFn) -> Result<TokenStream2> {
    if function.sig.asyncness.is_none() {
        return Err(syn::Error::new_spanned(
            function.sig.fn_token,
            "a scenario must be async",
        ));
    }
    let doc = description(function)?;
    let name = &function.sig.ident;
    let wrapper = format_ident!("__papo_scenario_{name}");
    let spec = format_ident!("__papo_scenario_spec_{name}");
    let (cap_names, caps): (Vec<_>, Vec<_>) = args
        .caps
        .unwrap_or_default()
        .into_iter()
        .map(|cap| {
            let text = cap.value();
            let (name, value) = text
                .split_once('=')
                .map_or((text.as_str(), None), |(name, value)| (name, Some(value)));
            let name_literal = LitStr::new(name, cap.span());
            let value = value.map_or_else(|| quote!(None), |value| quote!(Some(#value)));
            (
                name_literal,
                quote!(crate::CapRequirement { name: #name, value: #value }),
            )
        })
        .unzip();
    let isupport = args.isupport.unwrap_or_default();
    let covers = args.covers.unwrap_or_default();
    Ok(quote! {
        #function

        #(const _: () = assert!(papo_core::cap::is_known(#cap_names), "unknown capability");)*

        fn #wrapper(context: crate::Scenario) -> crate::ScenarioFuture {
            Box::pin(#name(context))
        }

        #[allow(non_upper_case_globals)]
        pub(crate) const #spec: crate::ScenarioSpec = crate::ScenarioSpec {
            name: stringify!(#name),
            doc: #doc,
            requires: crate::Requirements {
                caps: &[#(#caps),*],
                isupport: &[#(#isupport),*],
            },
            covers: &[#(#covers),*],
            run: #wrapper,
        };

        inventory::submit! {
            #spec
        }
    })
}

/// Declares and registers a conformance scenario.
#[proc_macro_attribute]
pub fn scenario(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as ScenarioArgs);
    let function = parse_macro_input!(input as ItemFn);
    expand(args, &function)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[cfg(test)]
mod tests {
    use quote::quote;

    use super::{ScenarioArgs, expand};

    #[test]
    fn parses_optional_capability_and_isupport_requirements() {
        let empty = syn::parse_str::<ScenarioArgs>("").unwrap();
        assert!(empty.caps.is_none());
        assert!(empty.isupport.is_none());
        assert!(empty.covers.is_none());

        let args = syn::parse_str::<ScenarioArgs>(
            r#"requires(caps = ["echo-message", "sasl=SCRAM-SHA-256"], isupport = ["MONITOR"]), covers = ["base.privmsg.channel"]"#,
        )
        .unwrap();
        assert_eq!(args.caps.unwrap()[1].value(), "sasl=SCRAM-SHA-256");
        assert_eq!(args.isupport.unwrap()[0].value(), "MONITOR");
        assert_eq!(args.covers.unwrap()[0].value(), "base.privmsg.channel");
    }

    #[test]
    fn rejects_unknown_and_duplicate_requirements() {
        for input in [
            r#"requires(tokens = ["WHOX"])"#,
            "other(caps = [])",
            "requires(caps = [], caps = [])",
            "covers = [], covers = []",
        ] {
            assert!(syn::parse_str::<ScenarioArgs>(input).is_err());
        }
    }

    #[test]
    fn expands_docs_cap_values_and_the_future_wrapper() {
        let args = syn::parse2(quote!(requires(caps = ["sasl=SCRAM-SHA-256"]))).unwrap();
        let function = syn::parse2(quote! {
            /// authenticates with a real SCRAM exchange
            async fn sasl_scram(context: Scenario) -> ScenarioResult { context.close().await }
        })
        .unwrap();
        let output = expand(args, &function).unwrap().to_string();
        assert!(output.contains("__papo_scenario_sasl_scram"));
        assert!(output.contains("__papo_scenario_spec_sasl_scram"));
        assert!(output.contains("authenticates with a real SCRAM exchange"));
        assert!(output.contains("SCRAM-SHA-256"));
        assert!(output.contains("papo_core :: cap :: is_known (\"sasl\")"));
    }

    #[test]
    fn rejects_undocumented_and_non_async_scenarios() {
        let undocumented = syn::parse2(quote!(
            async fn ping(_: Scenario) {}
        ))
        .unwrap();
        assert!(expand(ScenarioArgs::default(), &undocumented).is_err());
        let synchronous = syn::parse2(quote! {
            /// answers PING with PONG
            fn ping(_: Scenario) {}
        })
        .unwrap();
        assert!(expand(ScenarioArgs::default(), &synchronous).is_err());
    }
}
