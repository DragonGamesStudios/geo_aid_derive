extern crate proc_macro;
use proc_macro::TokenStream;

use syn::{parse_macro_input, DeriveInput, Path, Data, Fields, Expr, Ident, Attribute, parenthesized, Token, braced, Lit, DataEnum, Generics};
use quote::{format_ident, quote, ToTokens};
use syn::parse::{Parse, ParseStream};

#[proc_macro_derive(Evaluate, attributes(evaluate))]
pub fn derive_evaluate(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let output: Path = input
        .attrs
        .iter()
        .find(|x| x.path().is_ident("evaluate"))
        .unwrap()
        .parse_args()
        .unwrap();
    let enum_name = input.ident;

    let enum_data = match &input.data {
        Data::Enum(v) => v,
        _ => panic!("invalid evaluate input")
    };
    let variant1 = enum_data.variants.iter().map(|v| &v.ident);

    let expanded = quote!{
        impl Evaluate for #enum_name {
            type Output = #output;

            fn evaluate(&self, args: &EvaluationArgs) -> Self::Output {
                match self {
                    #(Self::#variant1(v) => v.evaluate(args),)*
                }
            }
        }
    };

    expanded.into()
}

#[proc_macro_derive(Kind, attributes(trivial, weigh_with, skip_collecting))]
pub fn derive_kind(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident;
    let generics = input.generics;
    let where_clause = generics.where_clause.clone();

    let expanded = match &input.data {
        Data::Struct(struct_data) => {
            let is_trivial = input.attrs.iter().find(|x| x.path().is_ident("trivial")).is_some();

            let (field1, field2) = match &struct_data.fields {
                Fields::Named(v) => {
                    (
                        v.named
                            .iter()
                            .filter(|f| f.attrs.iter().find(|x| x.path().is_ident("skip_collecting")).is_none())
                            .map(|f| &f.ident),
                        v.named
                            .iter()
                            .map(|field| {
                                let ident = &field.ident;

                                if let Some(weigh_with) = field.attrs.iter().find(|x| x.path().is_ident("weigh_with")) {
                                    let expr: Expr = weigh_with.parse_args().unwrap();

                                    quote!{ (#expr)(&self.#ident) }
                                } else {
                                    quote!{ self.#ident.weights }
                                }
                            })
                    )
                }
                Fields::Unnamed(_) => panic!("not supported"),
                Fields::Unit => panic!("not supported")
            };

            quote!{
                impl #generics Kind for #name #generics #where_clause {
                    fn collect(&self, exprs: &mut Vec<usize>) {
                        #(self.#field1.collect(exprs);)*
                    }

                    fn is_trivial(&self) -> bool {
                        #is_trivial
                    }

                    fn evaluate_weights(&self) -> Weights {
                        let mut weights = Weights::empty();
                        #(weights += &#field2;)*
                        weights
                    }
                }
            }
        }
        Data::Enum(enum_data) => {
            let variant1 = enum_data.variants.iter().map(|v| &v.ident);
            let variant2 = variant1.clone();
            let variant3 = variant1.clone();

            quote!{
                impl Kind for #name {
                    fn collect(&self, exprs: &mut Vec<usize>) {
                        match self {
                            #(Self::#variant1(v) => v.collect(exprs),)*
                        }
                    }

                    fn is_trivial(&self) -> bool {
                        match self {
                            #(Self::#variant2(v) => v.is_trivial(),)*
                        }
                    }

                    fn evaluate_weights(&self) -> Weights {
                        match self {
                            #(Self::#variant3(v) => v.evaluate_weights(),)*
                        }
                    }
                }
            }
        }
        Data::Union(_) => panic!("union not supported")
    };

    expanded.into()
}

enum DefinitionParam {
    Entity,
    NoEntity,
    Variable,
    Sequence,
    Map,
    Expression,
    Order(Expr)
}

impl DefinitionParam {
    #[must_use]
    pub fn is_entity(&self) -> bool {
        matches!(self, Self::Entity)
    }
}

impl Parse for DefinitionParam {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ident: Ident = input.parse()?;

        Ok(match ident.to_string().as_str() {
            "entity" => Self::Entity,
            "order" => Self::Order({
                let content;
                let _ = parenthesized!(content in input);

                content.parse()?
            }),
            "no_entity" => Self::NoEntity,
            "variable" => Self::Variable,
            "sequence" => Self::Sequence,
            "map" => Self::Map,
            &_ => panic!("invalid def")
        })
    }
}

impl From<&Vec<Attribute>> for DefinitionParam {
    fn from(value: &Vec<Attribute>) -> Self {
        value
            .iter()
            .find(|a| a.path().is_ident("def"))
            .map_or_else(
                || DefinitionParam::Expression,
                |x| x.parse_args().unwrap()
            )
    }
}

fn definition_handle_enum(name: &Ident, generics: &Generics, attrs: &Vec<Attribute>, enum_data: &DataEnum) -> TokenStream {
    let where_clause = &generics.where_clause;

    let variant1_code = enum_data.variants
        .iter()
        .map(|variant| {
            let name = &variant.ident;

            let field_ident = variant.fields
                .iter()
                .map(|f| {
                    if DefinitionParam::from(&f.attrs).is_entity() {
                        format_ident!("id")
                    } else {
                        format_ident!("_")
                    }
                });

            let field_getter = if let DefinitionParam::Order(order) = DefinitionParam::from(attrs) {
                quote! {#order}
            } else {
                variant.fields
                    .iter()
                    .find(|f| {
                        DefinitionParam::from(&f.attrs).is_entity()
                    })
                    .map_or_else(
                        || quote! {0},
                        |_| quote! {
                            context.get_entity(*id).order(context)
                        }
                    )
            };

            let fields = if variant.fields.is_empty() {
                quote! {}
            } else {
                quote! {(#(#field_ident),*)}
            };

            quote! {
                Self::#name #fields => {
                    #field_getter
                }
            }
        });

    let variant2_code = enum_data.variants
        .iter()
        .map(|variant| {
            let name = &variant.ident;

            let field_ident = (0..variant.fields.len())
                .into_iter()
                .map(|i| {
                    format_ident!("v{i}")
                });

            let field_checker = variant.fields
                .iter()
                .enumerate()
                .map(|(i, field)| {
                    let field_ident = format_ident!("v{i}");
                    let field_def = DefinitionParam::from(&field.attrs);

                    match field_def {
                        DefinitionParam::Entity => quote! {
                            (if *#field_ident == entity {
                                true
                            } else {
                                context.get_entity(*#field_ident).contains_entity(entity, context)
                            })
                        },
                        DefinitionParam::Variable => quote! {
                            #field_ident.borrow().definition.contains_entity(entity, context)
                        },
                        DefinitionParam::Sequence => quote! {
                            #field_ident.iter().any(|x| x.contains_entity(entity, context))
                        },
                        DefinitionParam::Map => quote! {
                            #field_ident.values().any(|x| x.contains_entity(entity, context))
                        },
                        DefinitionParam::NoEntity
                        | DefinitionParam::Order(_) => quote! {
                            false
                        },
                        DefinitionParam::Expression => quote! {
                            #field_ident.contains_entity(entity, context)
                        }
                    }
                });

            let fields = if variant.fields.is_empty() {
                quote! {}
            } else {
                quote! {(#(#field_ident),*)}
            };

            quote! {
                Self::#name #fields => {
                    #(#field_checker ||)* false
                }
            }
        });

    let expanded = quote! {
        impl #generics Definition for #name #generics #where_clause {
            fn order(&self, context: &CompileContext) -> usize {
                match self {
                    #(#variant1_code)*
                }
            }

            fn contains_entity(&self, entity: usize, context: &CompileContext) -> bool {
                match self {
                    #(#variant2_code)*
                }
            }
        }
    };
    // panic!("{}", expanded.to_string());

    expanded.into()
}

#[proc_macro_derive(Definition, attributes(def))]
pub fn derive_definition(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let name = &input.ident;
    let generics = &input.generics;

    match &input.data {
        Data::Enum(v) => definition_handle_enum(name, generics, &input.attrs, v),
        Data::Struct(struct_data) => {
            let order_field_code = struct_data.fields
                .iter()
                .map(|field| {
                    if DefinitionParam::from(&field.attrs).is_entity() {
                        let field_name = field.ident.as_ref().unwrap();
                        quote! {
                            self.#field_name.order(context)
                        }
                    } else {
                        quote! {}
                    }
                });

            let contains_field_code = struct_data.fields
                .iter()
                .map(|field| {
                    if DefinitionParam::from(&field.attrs).is_entity() {
                        let field_name = field.ident.as_ref().unwrap();
                        quote! {
                            self.#field_name.contains_entity(entity, context)
                        }
                    } else {
                        quote! {}
                    }
                });

            let expanded = quote! {
                impl Definition for #name {
                    fn order(&self, context: &CompileContext) -> usize {
                        #(#order_field_code)*
                    }

                    fn contains_entity(&self, entity: usize, context: &CompileContext) -> bool {
                        #(#contains_field_code)*
                    }
                }
            };

            expanded.into()
        }
        _ => panic!("unsupported")
    }
}

enum GType {
    Simple(Ident),
    Collection(usize),
    Bundle(String)
}

// impl GType {
//     fn get_conversion_target(&self) -> Ident {
//         match self {
//             GType::Simple(s) => match s.to_string().as_str() {
//                 "DISTANCE"
//                 | "ANGLE"
//                 | "SCALAR" => format_ident!("Scalar"),
//                 "POINT" => format_ident!("Point"),
//                 "CIRCLE" => format_ident!("Circle"),
//                 "LINE" => format_ident!("Line"),
//                 &_ => unreachable!()
//             },
//             GType::Collection(_) => format_ident!("PointCollection"),
//             GType::Bundle(_) => format_ident!("Bundle")
//         }
//     }
// }

struct OverloadFunction {
    params: Vec<GType>,
    param_group: Option<GType>,
    return_type: GType,
    func: Expr
}

struct OverloadRule {
    lhs: GType,
    rhs: GType,
    func: Expr
}

enum OverloadInput {
    Function(OverloadFunction),
    Rule(OverloadRule)
}

impl ToTokens for GType {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        match self {
            GType::Simple(ident) => tokens.extend(quote! {
                crate::script::ty::#ident
            }),
            GType::Collection(l) => tokens.extend(quote! {
                crate::script::ty::collection(#l)
            }),
            GType::Bundle(name) => tokens.extend(quote! {
                crate::script::ty::bundle(#name)
            }),
        }
    }
}

impl GType {
    fn parse(input: ParseStream) -> Option<Self> {
        if !input.peek(Ident) && !input.peek(Lit) {
            return None;
        }

        if let Some(ident) = input.parse::<Option<Ident>>().ok()? {
            match ident.to_string().as_str() {
                "DISTANCE"
                | "ANGLE"
                | "SCALAR"
                | "POINT"
                | "LINE"
                | "CIRCLE"=> Some(Self::Simple(ident)),
                other => Some(Self::Bundle(other.to_string()))
            }
        } else {
            let l = input.parse::<Lit>().ok()?;
            let _ = input.parse::<Token![-]>().ok()?;
            let ident = input.parse::<Ident>().ok()?;
            if ident.to_string().as_str() == "P" {
                Some(Self::Collection(match l {
                    Lit::Int(i) => i.base10_parse().unwrap(),
                    _ => panic!("WRONG")
                }))
            } else {
                panic!("WRONG")
            }
        }
    }
}

impl Parse for OverloadInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        if let Some(lhs) = GType::parse(input) {
            let _: Ident = input.parse()?;
            let rhs = GType::parse(input).unwrap();

            let _: Token![:] = input.parse()?;
            let func = input.parse()?;

            Ok(OverloadInput::Rule(OverloadRule {
                lhs,
                rhs,
                func
            }))
        } else {
            let content;
            let _ = parenthesized!(content in input);

            let mut params = Vec::new();
            while let Some(param) = GType::parse(&content) {
                params.push(param);

                if content.parse::<Option<Token![,]>>()?.is_none() {
                    break
                }
            }

            let param_group = if !content.is_empty() {
                let _: Token![.] = content.parse()?;
                let _: Token![.] = content.parse()?;
                let _: Token![.] = content.parse()?;
                Some(GType::parse(&content).unwrap())
            } else {
                None
            };

            let _: Token![->] = input.parse()?;
            let return_type = GType::parse(input).unwrap();

            let func = if input.parse::<Option<Token![:]>>()?.is_some() {
                input.parse()?
            } else {
                let content;
                let _ = braced!(content in input);

                content.parse()?
            };

            Ok(OverloadInput::Function(OverloadFunction {
                params,
                param_group,
                return_type,
                func
            }))
        }
    }
}

#[proc_macro]
pub fn overload(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as OverloadInput);

    match input {
        OverloadInput::Function(OverloadFunction{
            params,
            param_group,
            return_type,
            func
        }) => {
            let params_it = params.iter();

            let converted_it = params.iter().map(|_| {
                quote! {
                    &crate::script::unroll::Convert::convert(args.next().unwrap()).unwrap(),
                }
            });

            let converted_group = param_group.as_ref().map(|_| quote! {
                &args.map(|x| crate::script::unroll::Convert::convert(x).unwrap()).collect::<Vec<_>>(),
            }).into_iter();

            let param_group_it = param_group.as_ref().map_or_else(
                || quote! {None},
                |x| quote! {Some(#x)}
            );

            let expanded = quote! {
                crate::script::unroll::FunctionOverload {
                    returned_type: #return_type,
                    definition: crate::script::unroll::FunctionDefinition(Box::new(
                        |args, context, display| {
                            let mut args = args.into_iter().cloned();
                            crate::script::unroll::AnyExpr::from((#func)(
                                #(#converted_it)*
                                #(#converted_group)*
                                context,
                                display
                            ))
                        }
                    )),
                    params: vec![#(#params_it),*],
                    param_group: #param_group_it
                }
            };

            expanded.into()
        }
        OverloadInput::Rule(OverloadRule {
            lhs,
            rhs,
            func
                            }) => {
            let expanded = quote! {
                crate::script::unroll::RuleOverload {
                    definition: crate::script::unroll::RuleDefinition(Box::new(
                        |lhs, rhs, context, properties| {
                            (#func)(
                                &crate::script::unroll::Convert::convert(lhs.clone()).unwrap(),
                                &crate::script::unroll::Convert::convert(rhs.clone()).unwrap(),
                                context,
                                properties
                            )
                        }
                    )),
                    params: (
                        #lhs,
                        #rhs
                    )
                }
            };

            expanded.into()
        }
    }
}
