// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use itertools::Itertools;
use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DataEnum, DeriveInput, parse_macro_input};

/// Derives `ProjectRead` for an enum by delegating to its variants and
/// synthesizing unified associated types.
///
/// This macro implements `ProjectRead` for an enum whose variants each
/// contain a type that already implements `ProjectRead`. The derived
/// implementation delegates all trait methods to the active variant.
///
/// In addition, the macro generates two new enums to unify associated types
/// across variants:
///
/// - `<EnumName>Error`
/// - `<EnumName>SourceReader<'a>`
///
/// These enums contain one variant per original enum variant and wrap the
/// corresponding associated types from each inner `ProjectRead` implementation.
///
/// # Generated Associated Types
///
/// The derived implementation defines:
///
/// - `type Error = <EnumName>Error`
/// - `type SourceReader<'a> = <EnumName>SourceReader<'a>`
///
/// where:
///
/// - `<EnumName>Error` is an enum with one variant per original enum variant,
///   wrapping that variant’s `ProjectRead::Error` type.
/// - `<EnumName>SourceReader<'a>` is an enum with one variant per original
///   enum variant, wrapping that variant’s `ProjectRead::SourceReader<'a>`
///   type.
///
/// This allows each variant to use its own concrete error and reader types,
/// while presenting a single unified `ProjectRead` implementation for the
/// outer enum.
///
/// # Method Delegation
///
/// - [`ProjectRead::get_project`] delegates to the active variant, mapping
///   errors into `<EnumName>Error`.
/// - [`ProjectRead::read_source`] delegates to the active variant and wraps
///   the returned reader in `<EnumName>SourceReader<'_>`.
/// - [`ProjectRead::sources`] delegates directly to the active variant.
/// - [`ProjectRead::get_info`], [`ProjectRead::get_meta`],
///   [`ProjectRead::version`], and [`ProjectRead::usage`] delegate directly to
///   the active variant so leaf overrides survive wrapper enums.
/// - [`ProjectRead::checksum_canonical_hex`] delegates to the active variant.
///
/// All other methods are handled by the default implementation of the
/// `ProjectRead` trait.
///
/// All delegation is performed via a `match` on `self`. No dynamic dispatch
/// is introduced.
///
/// # Requirements
///
/// - Each variant must contain exactly one value whose type implements
///   `ProjectRead`.
/// - No additional fields are permitted in a variant.
/// - The enum may be generic, provided the generics are compatible with the
///   synthesized associated types.
///
/// # Design Rationale
///
/// This derive is useful when modeling multiple project backends behind a
/// single enum type while preserving static dispatch and allowing each
/// backend to retain its own concrete error and reader types.
///
/// The generated implementation is zero-cost beyond the enum match required
/// for delegation and wrapping.
#[proc_macro_derive(ProjectRead)]
pub fn project_read_derive(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);

    let Data::Enum(DataEnum { variants, .. }) = &ast.data else {
        return syn::Error::new_spanned(&ast.ident, "ProjectRead can only be derived on an enum")
            .to_compile_error()
            .into();
    };

    let (impl_generics, type_generics, where_clause) = ast.generics.split_for_impl();
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
                // variant_list
                quote! {
                    #variant_ident
                },
                // error_variants
                quote! {
                    #[error(transparent)]
                    #variant_ident(#variant_ident)
                },
                // error_args
                quote! {
                    <#variant_type as ProjectRead>::Error
                },
                // source_reader_variants
                quote! {
                    #variant_ident(#variant_ident)
                },
                // variants_read
                quote! {
                    #variant_ident: ::std::io::Read
                },
                // source_reader_match
                quote! {
                    #source_reader_ident::#variant_ident(reader) => reader.read(buf)
                },
                // source_reader_args
                quote! {
                    <#variant_type as ProjectRead>::SourceReader<'a>
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
                    #enum_ident::#variant_ident(project) => project.sources(ctx)
                        .map_err(#error_ident::#variant_ident)
                },
                // get_info_match
                quote! {
                    #enum_ident::#variant_ident(project) => project
                        .get_info()
                        .map_err(#error_ident::#variant_ident)
                },
                // get_meta_match
                quote! {
                    #enum_ident::#variant_ident(project) => project
                        .get_meta()
                        .map_err(#error_ident::#variant_ident)
                },
                // version_match
                quote! {
                    #enum_ident::#variant_ident(project) => project
                        .version()
                        .map_err(#error_ident::#variant_ident)
                },
                // usage_match
                quote! {
                    #enum_ident::#variant_ident(project) => project
                        .usage()
                        .map_err(#error_ident::#variant_ident)
                },
                // checksum_canonical_hex_match — forward so that any leaf
                // override (e.g. a remote-index project with a prefetched
                // digest) isn't bypassed by the trait default.
                quote! {
                    #enum_ident::#variant_ident(project) => project
                        .checksum_canonical_hex()
                        .map_err(|e| e.map_project_read(#error_ident::#variant_ident))
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

    // Manual loop instead of `multiunzip` because the 15-element tuple
    // exceeds the arity limit of `itertools::Itertools::multiunzip`.
    let mut variant_list = vec![];
    let mut error_variants = vec![];
    let mut error_args = vec![];
    let mut source_reader_variants = vec![];
    let mut variants_read = vec![];
    let mut source_reader_match = vec![];
    let mut source_reader_args = vec![];
    let mut get_project_match = vec![];
    let mut read_source_match = vec![];
    let mut sources_match = vec![];
    let mut get_info_match = vec![];
    let mut get_meta_match = vec![];
    let mut version_match = vec![];
    let mut usage_match = vec![];
    let mut checksum_canonical_hex_match = vec![];

    for (
        variant_list_part,
        error_variants_part,
        error_args_part,
        source_reader_variants_part,
        variants_read_part,
        source_reader_match_part,
        source_reader_args_part,
        get_project_match_part,
        read_source_match_part,
        sources_match_part,
        get_info_match_part,
        get_meta_match_part,
        version_match_part,
        usage_match_part,
        checksum_canonical_hex_match_part,
    ) in variant_parts.iter().cloned()
    {
        variant_list.push(variant_list_part);
        error_variants.push(error_variants_part);
        error_args.push(error_args_part);
        source_reader_variants.push(source_reader_variants_part);
        variants_read.push(variants_read_part);
        source_reader_match.push(source_reader_match_part);
        source_reader_args.push(source_reader_args_part);
        get_project_match.push(get_project_match_part);
        read_source_match.push(read_source_match_part);
        sources_match.push(sources_match_part);
        get_info_match.push(get_info_match_part);
        get_meta_match.push(get_meta_match_part);
        version_match.push(version_match_part);
        usage_match.push(usage_match_part);
        checksum_canonical_hex_match.push(checksum_canonical_hex_match_part);
    }

    let expanded = quote! {
        #[derive(::std::fmt::Debug, ::thiserror::Error)]
        pub enum #error_ident<
            #( #variant_list ),*
        > {
            #( #error_variants ),*
        }

        pub enum #source_reader_ident<
            #( #variant_list ),*
        > {
            #( #source_reader_variants ),*
        }

        impl<
            #( #variants_read ),*
        > ::std::io::Read
        for #source_reader_ident<
            #( #variant_list ),*
        > {
            fn read(&mut self, buf: &mut [u8]) -> ::std::io::Result<usize> {
                match self {
                    #( #source_reader_match ),*
                }
            }
        }

        impl #impl_generics ProjectRead for #enum_ident #type_generics #where_clause {
            type Error = #error_ident<
                #( #error_args ),*
            >;

            fn get_project(
                &self,
            ) -> ::std::result::Result<
                (
                    ::std::option::Option<InterchangeProjectInfoRaw>,
                    ::std::option::Option<InterchangeProjectMetadataRaw>,
                ),
                Self::Error,
            > {
                match self {
                    #( #get_project_match ),*
                }
            }

            type SourceReader<'a>
                = #source_reader_ident<
                    #( #source_reader_args ),*
                >
            where
                Self: 'a;

            fn read_source<P: ::std::convert::AsRef<Utf8UnixPath>>(
                &self,
                path: P,
            ) -> ::std::result::Result<Self::SourceReader<'_>, Self::Error> {
                match self {
                    #( #read_source_match ),*
                }
            }

            fn sources(&self, ctx: &ProjectContext) -> ::std::result::Result<::std::vec::Vec<Source>, Self::Error> {
                match self {
                    #( #sources_match ),*
                }
            }

            fn get_info(
                &self,
            ) -> ::std::result::Result<::std::option::Option<InterchangeProjectInfoRaw>, Self::Error> {
                match self {
                    #( #get_info_match ),*
                }
            }

            fn get_meta(
                &self,
            ) -> ::std::result::Result<::std::option::Option<InterchangeProjectMetadataRaw>, Self::Error> {
                match self {
                    #( #get_meta_match ),*
                }
            }

            fn version(
                &self,
            ) -> ::std::result::Result<::std::option::Option<::std::string::String>, Self::Error> {
                match self {
                    #( #version_match ),*
                }
            }

            fn usage(
                &self,
            ) -> ::std::result::Result<
                ::std::option::Option<
                    ::std::vec::Vec<::sysand_core::model::InterchangeProjectUsageRaw>,
                >,
                Self::Error,
            > {
                match self {
                    #( #usage_match ),*
                }
            }

            fn checksum_canonical_hex(
                &self,
            ) -> ::std::result::Result<
                ::std::option::Option<::std::string::String>,
                CanonicalizationError<Self::Error>,
            > {
                match self {
                    #( #checksum_canonical_hex_match ),*
                }
            }
        }
    };

    TokenStream::from(expanded)
}

/// Derives `ProjectMut` for an enum by delegating to its variants.
///
/// This macro implements `ProjectMut` for an enum whose variants each
/// contain a type that already implements `ProjectMut`. All trait methods
/// are delegated to the active variant.
///
/// Because `ProjectMut` extends `ProjectRead`, this derive requires
/// that the enum also implement `ProjectRead`. In typical usage, this is
/// provided by the corresponding [`ProjectRead`] derive macro.
///
/// # Associated Types
///
/// This derive does **not** introduce new associated types.
///
/// Instead, it reuses the `Error` type defined by the enum’s
/// `ProjectRead` implementation (typically the synthesized
/// `<EnumName>Error` type generated by the [`ProjectRead`] derive).
///
/// All errors produced by delegated methods are forwarded unchanged.
///
/// # Method Delegation
///
/// - [`ProjectMut::put_info`] delegates to the active variant.
/// - [`ProjectMut::put_meta`] delegates to the active variant.
/// - [`ProjectMut::write_source`] delegates to the active variant.
///
/// All other methods are handled by the default implementation of the
/// `ProjectMut` trait.
///
/// Delegation is implemented via a `match` on `self`. No dynamic dispatch
/// is introduced.
///
/// # Requirements
///
/// - Each variant must contain exactly one value whose type implements
///   `ProjectMut`.
/// - The enum must also implement `ProjectRead` (typically via the
///   corresponding derive macro).
/// - No additional fields are permitted in a variant.
///
/// # Design Rationale
///
/// This derive enables modeling multiple mutable project backends behind
/// a single enum while preserving static dispatch and maintaining a unified
/// error type.
///
/// The generated implementation is zero-cost beyond the enum match required
/// for delegation.
#[proc_macro_derive(ProjectMut)]
pub fn project_mut_derive(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);

    let Data::Enum(DataEnum { variants, .. }) = &ast.data else {
        return syn::Error::new_spanned(&ast.ident, "ProjectMut can only be derived on an enum")
            .to_compile_error()
            .into();
    };

    let (impl_generics, type_generics, where_clause) = ast.generics.split_for_impl();
    let enum_ident = &ast.ident;
    let error_ident = syn::Ident::new(format!("{}Error", enum_ident).as_str(), enum_ident.span());

    let variant_parts: Result<Vec<_>, _> = variants
        .iter()
        .map(|variant| {
            let variant_ident = variant.ident.clone();
            match &variant.fields {
                syn::Fields::Unnamed(fields) if fields.unnamed.len() != 1 => {
                    return Err(syn::Error::new_spanned(
                        &variant.ident,
                        "each variant must contain exactly one field",
                    ));
                }
                syn::Fields::Unnamed(_) => {}
                _ => {
                    return Err(syn::Error::new_spanned(
                        &variant.ident,
                        "only tuple variants supported",
                    ));
                }
            };
            Ok((
                // put_info_match
                quote! {
                    #enum_ident::#variant_ident(project) => project
                        .put_info(info, overwrite)
                        .map_err(#error_ident::#variant_ident)
                },
                // put_meta_match
                quote! {
                    #enum_ident::#variant_ident(project) => project
                        .put_meta(meta, overwrite)
                        .map_err(#error_ident::#variant_ident)
                },
                // write_source_match
                quote! {
                    #enum_ident::#variant_ident(project) => project
                        .write_source(path, source, overwrite)
                        .map_err(#error_ident::#variant_ident)
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

    let (put_info_match, put_meta_match, write_source_match): (Vec<_>, Vec<_>, Vec<_>) =
        variant_parts.iter().cloned().multiunzip();

    let expanded = quote! {
        impl #impl_generics ProjectMut for #enum_ident #type_generics #where_clause {
            fn put_info(
                &mut self,
                info: &InterchangeProjectInfoRaw,
                overwrite: bool,
            ) -> ::std::result::Result<(), Self::Error> {
                match self {
                    #( #put_info_match ),*
                }
            }

            fn put_meta(
                &mut self,
                meta: &InterchangeProjectMetadataRaw,
                overwrite: bool,
            ) -> ::std::result::Result<(), Self::Error> {
                match self {
                    #( #put_meta_match ),*
                }
            }

            fn write_source<P: ::std::convert::AsRef<Utf8UnixPath>, R: ::std::io::Read>(
                &mut self,
                path: P,
                source: &mut R,
                overwrite: bool,
            ) -> ::std::result::Result<(), Self::Error> {
                match self {
                    #( #write_source_match ),*
                }
            }
        }
    };

    TokenStream::from(expanded)
}
