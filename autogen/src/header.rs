use crate::structs;
use crate::utils::*;

use heck::{ShoutySnakeCase, SnakeCase};
use proc_macro2::TokenStream;
use quote::quote;
use std::collections::BTreeMap;

/// Returns the markdown string containing a link to the spec for the given
/// operand `kind`.
fn get_spec_link(kind: &str) -> String {
    let symbol = kind.to_snake_case();
    format!(
        "[{text}]({link})",
        text = kind,
        link = format!(
            "https://www.khronos.org/registry/spir-v/\
                            specs/unified1/SPIRV.html#_a_id_{}_a_{}",
            symbol, symbol
        )
    )
}

fn value_enum_attribute() -> TokenStream {
    quote! {
        #[repr(u32)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[cfg_attr(feature = "serialize", derive(serde::Serialize))]
        #[cfg_attr(feature = "deserialize", derive(serde::Deserialize))]
    }
}

fn bit_enum_attribute() -> TokenStream {
    quote! {
        #[cfg_attr(feature = "serialize", derive(serde::Serialize))]
        #[cfg_attr(feature = "deserialize", derive(serde::Deserialize))]
    }
}

fn from_primitive_impl(from_prim: &[TokenStream], kind: &proc_macro2::Ident) -> TokenStream {
    quote! {
        impl num_traits::FromPrimitive for #kind {
            #[allow(trivial_numeric_casts)]
            fn from_i64(n: i64) -> Option<Self> {
                Some(match n as u32 {
                    #(#from_prim,)*
                    _ => return None
                })
            }

            fn from_u64(n: u64) -> Option<Self> {
                Self::from_i64(n as i64)
            }
        }
    }
}

fn gen_bit_enum_operand_kind(grammar: &structs::OperandKind) -> TokenStream {
    let mut elements = vec![];
    let mut operands = vec![];
    let mut additional_operands_list = vec![];

    for enumerant in grammar.enumerants.iter() {
        // Special treatment for "NaN"
        let symbol = as_ident(
            &enumerant
                .symbol
                .to_shouty_snake_case()
                .replace("NA_N", "NAN"),
        );
        let value = enumerant.value;

        elements.push(quote! {
            const #symbol = #value;
        });

        let parameters = enumerant.parameters.iter().map(|op| {
            let kind = as_ident(&op.kind);

            let quant = match op.quantifier {
                structs::Quantifier::One => quote! { OperandQuantifier::One },
                structs::Quantifier::ZeroOrOne => quote! { OperandQuantifier::ZeroOrOne },
                structs::Quantifier::ZeroOrMore => quote! { OperandQuantifier::ZeroOrMore },
            };

            quote! {
                LogicalOperand {
                    kind: OperandKind::#kind,
                    quantifier: #quant
                }
            }
        });

        if !enumerant.parameters.is_empty() {
            additional_operands_list.push(quote! { Self::#symbol });

            operands.push(quote! {
                Self::#symbol if self.contains(*v) => {
                    [#( #parameters ),*].iter()
                }
            });
        }
    }

    let comment = format!("SPIR-V operand kind: {}", get_spec_link(&grammar.kind));
    let kind = as_ident(&grammar.kind);
    let attribute = bit_enum_attribute();

    quote! {
        bitflags! {
            #[doc = #comment]
            #attribute
            pub struct #kind: u32 {
                #(#elements)*
            }
        }
    }
}

fn gen_value_enum_operand_kind(grammar: &structs::OperandKind) -> TokenStream {
    let kind = as_ident(&grammar.kind);

    // We can have more than one enumerants mapping to the same discriminator.
    // Use associated constants for these aliases.
    let mut seen_discriminator = BTreeMap::new();
    let mut enumerants = vec![];
    let mut from_prim_list = vec![];
    let mut aliases = vec![];
    let mut capability_clauses = BTreeMap::new();
    let mut extension_clauses = BTreeMap::new();
    let mut operand_clauses = BTreeMap::new();
    let mut from_str_impl = vec![];
    for e in &grammar.enumerants {
        if let Some(discriminator) = seen_discriminator.get(&e.value) {
            let name_str = &e.symbol;
            let symbol = as_ident(&e.symbol);
            aliases.push(quote! {
                pub const #symbol: Self = Self::#discriminator;
            });
            from_str_impl.push(quote! { #name_str => Ok(Self::#discriminator), });
        } else {
            // Special case for Dim. Its enumerants can start with a digit.
            // So prefix with the kind name here.
            let name_str = if grammar.kind == "Dim" {
                let mut name = "Dim".to_string();
                name.push_str(&e.symbol);
                name
            } else {
                e.symbol.to_string()
            };
            let name = as_ident(&name_str);
            let number = e.value;
            seen_discriminator.insert(e.value, name.clone());
            enumerants.push(quote! { #name = #number });
            from_prim_list.push(quote! { #number => Self::#name });
            from_str_impl.push(quote! { #name_str => Ok(Self::#name), });

            capability_clauses
                .entry(&e.capabilities)
                .or_insert_with(Vec::new)
                .push(name.clone());

            extension_clauses
                .entry(&e.extensions)
                .or_insert_with(Vec::new)
                .push(name.clone());

            operand_clauses
                .entry(name.clone())
                .or_insert_with(Vec::new)
                .extend(e.parameters.iter().map(|op| {
                    let kind = as_ident(&op.kind);

                    let quant = match op.quantifier {
                        structs::Quantifier::One => quote! { OperandQuantifier::One },
                        structs::Quantifier::ZeroOrOne => quote! { OperandQuantifier::ZeroOrOne },
                        structs::Quantifier::ZeroOrMore => quote! { OperandQuantifier::ZeroOrMore },
                    };

                    quote! {
                        LogicalOperand {
                            kind: OperandKind::#kind,
                            quantifier: #quant
                        }
                    }
                }));
        }
    }

    let comment = format!("/// SPIR-V operand kind: {}", get_spec_link(&grammar.kind));
    let attribute = value_enum_attribute();

    let from_prim_impl = from_primitive_impl(&from_prim_list, &kind);

    quote! {
        #[doc = #comment]
        #attribute
        #[allow(clippy::upper_case_acronyms)]
        pub enum #kind {
            #(#enumerants),*
        }

        #[allow(non_upper_case_globals)]
        impl #kind {
            #(#aliases)*
        }

        #from_prim_impl

        impl core::str::FromStr for #kind {
            type Err = ();

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    #(#from_str_impl)*
                    _ => Err(()),
                }
            }
        }
    }
}

/// Returns the code defining the enum for an operand kind by parsing
/// the given SPIR-V `grammar`.
fn gen_operand_kind(grammar: &structs::OperandKind) -> Option<TokenStream> {
    use structs::Category::*;
    match grammar.category {
        BitEnum => Some(gen_bit_enum_operand_kind(grammar)),
        ValueEnum => Some(gen_value_enum_operand_kind(grammar)),
        _ => None,
    }
}

/// Returns the generated SPIR-V header.
pub fn gen_spirv_header(grammar: &structs::Grammar) -> TokenStream {
    // constants and types.
    let magic_number = format!("{:#010X}", grammar.magic_number)
        .parse::<TokenStream>()
        .unwrap();
    let major_version = grammar.major_version;
    let minor_version = grammar.minor_version;
    let revision = grammar.revision;

    // Operand kinds.
    let kinds = grammar.operand_kinds.iter().filter_map(gen_operand_kind);

    // Opcodes.

    // We can have more than one op symbol mapping to the same opcode.
    // Use associated constants for these aliases.
    let mut seen_discriminator = BTreeMap::new();
    let mut opcodes = vec![];
    let mut aliases = vec![];
    let mut from_prim_list = vec![];

    // Get the instruction table.
    for inst in &grammar.instructions {
        // Omit the "Op" prefix.
        let opname = as_ident(&inst.opname[2..]);
        let opcode = inst.opcode;
        if let Some(discriminator) = seen_discriminator.get(&opcode) {
            aliases.push(quote! { pub const #opname : Op = Op::#discriminator; });
        } else {
            opcodes.push(quote! { #opname = #opcode });
            from_prim_list.push(quote! { #opcode => Op::#opname });
            seen_discriminator.insert(opcode, opname.clone());
        }
    }

    let comment = format!("SPIR-V {} opcodes", get_spec_link("instructions"));
    let attribute = value_enum_attribute();
    let from_prim_impl = from_primitive_impl(&from_prim_list, &as_ident("Op"));

    quote! {
        //pub use crate::grammar::{OperandKind, OperandQuantifier, LogicalOperand};
        pub type Word = u32;
        pub const MAGIC_NUMBER: u32 = #magic_number;
        pub const MAJOR_VERSION: u8 = #major_version;
        pub const MINOR_VERSION: u8 = #minor_version;
        pub const REVISION: u8 = #revision;

        #(#kinds)*

        #[doc = #comment]
        #attribute
        #[allow(clippy::upper_case_acronyms)]
        pub enum Op {
            #(#opcodes),*
        }

        #[allow(clippy::upper_case_acronyms)]
        #[allow(non_upper_case_globals)]
        impl Op {
            #(#aliases)*
        }

        #from_prim_impl
    }
}

/// Returns extended instruction opcodes
pub fn gen_opcodes(op: &str, grammar: &structs::ExtInstSetGrammar, comment: &str) -> TokenStream {
		let op = as_ident(op);
    // Get the instruction table
    let opcodes = grammar.instructions.iter().map(|inst| {
        // Omit the "Op" prefix.
        let opname = as_ident(&inst.opname);
        let opcode = inst.opcode;
        quote! { #opname = #opcode }
    });

    let from_prim_list = grammar
        .instructions
        .iter()
        .map(|inst| {
            let opname = as_ident(&inst.opname);
            let opcode = inst.opcode;
            quote! { #opcode => #op::#opname }
        })
        .collect::<Vec<_>>();

    let attribute = value_enum_attribute();
    let from_prim_impl = from_primitive_impl(&from_prim_list, &op);

    quote! {
        #[doc = #comment]
        #attribute
        #[allow(clippy::upper_case_acronyms)]
        pub enum #op {
            #(#opcodes),*
        }

        #from_prim_impl
    }
}
