// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use itertools::Itertools;
use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DataEnum, DeriveInput, parse_macro_input};

#[proc_macro_derive(ProjectRead)]
pub fn project_read_derive(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);

    // This derive only works on an enum
    let Data::Enum(DataEnum { variants, .. }) = &ast.data else {
        return syn::Error::new_spanned(&ast.ident, "ProjectRead can only be derived on an enum")
            .to_compile_error()
            .into();
    };

    let enum_ident = &ast.ident;
    let error_ident = syn::Ident::new(format!("{}Error", enum_ident).as_str(), enum_ident.span());
    let source_reader_ident = syn::Ident::new(
        format!("{}SourceReader", enum_ident).as_str(),
        enum_ident.span(),
    );

    let variant_parts: Result<Vec<_>, _> = variants
        .iter()
        .map(|variant| {
            let variant_ident = variant.ident.clone();
            let variant_type = match &variant.fields {
                syn::Fields::Unnamed(fields) if fields.unnamed.len() != 1 => {
                    return Err(syn::Error::new_spanned(
                        &variant.ident,
                        "each variant must contain exactly one field",
                    ));
                }
                syn::Fields::Unnamed(fields) => fields.unnamed.first().unwrap().ty.clone(),
                _ => {
                    return Err(syn::Error::new_spanned(
                        &variant.ident,
                        "only tuple variants supported",
                    ));
                }
            };
            Ok((
                // error_variants
                quote! {
                    #[error(transparent)]
                    #variant_ident(<#variant_type as ProjectRead>::Error)
                },
                // source_reader_variants
                quote! {
                    #variant_ident(<#variant_type as ProjectRead>::SourceReader<'a>)
                },
                // source_reader_match
                quote! {
                    #source_reader_ident::#variant_ident(reader) => reader.read(buf)
                },
                // get_project_match
                quote! {
                    #enum_ident::#variant_ident(project) => project
                        .get_project()
                        .map_err(#error_ident::#variant_ident)
                },
                // read_source_match
                quote! {
                    #enum_ident::#variant_ident(project) => project
                        .read_source(path)
                        .map(#source_reader_ident::#variant_ident)
                        .map_err(#error_ident::#variant_ident)
                },
                // sources_match
                quote! {
                    #enum_ident::#variant_ident(project) => project.sources()
                },
            ))
        })
        .collect();

    let variant_parts = match variant_parts {
        Ok(var) => var,
        Err(err) => {
            return err.to_compile_error().into();
        }
    };

    let (
        error_variants,
        source_reader_variants,
        source_reader_match,
        get_project_match,
        read_source_match,
        sources_match,
    ): (Vec<_>, Vec<_>, Vec<_>, Vec<_>, Vec<_>, Vec<_>) =
        variant_parts.iter().cloned().multiunzip();

    let expanded = quote! {
        #[derive(Debug, thiserror::Error)]
        pub enum #error_ident {
            #( #error_variants ),*
        }

        pub enum #source_reader_ident<'a>
        where
            Self: 'a,
        {
            #( #source_reader_variants ),*
        }

        impl std::io::Read for #source_reader_ident<'_> {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                match self {
                    #( #source_reader_match ),*
                }
            }
        }

        impl ProjectRead for #enum_ident {
            type Error = #error_ident;

            fn get_project(
                &self,
            ) -> Result<
                (
                    Option<InterchangeProjectInfoRaw>,
                    Option<InterchangeProjectMetadataRaw>,
                ),
                Self::Error,
            > {
                match self {
                    #( #get_project_match ),*
                }
            }

            type SourceReader<'a>
                = #source_reader_ident<'a>
            where
                Self: 'a;

            fn read_source<P: AsRef<typed_path::Utf8UnixPath>>(
                &self,
                path: P,
            ) -> Result<Self::SourceReader<'_>, Self::Error> {
                match self {
                    #( #read_source_match ),*
                }
            }

            fn sources(&self) -> Vec<Source> {
                match &self {
                    #( #sources_match ),*
                }
            }
        }
    };

    TokenStream::from(expanded)
}
