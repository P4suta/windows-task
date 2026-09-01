//! Proc macro used by `windows-task` handler implementations.

use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemImpl, LitStr, Token, parse::Parser, parse_macro_input, punctuated::Punctuated};

/// Marks a `TaskHandler` implementation, emits its CLSID, and generates the
/// in-process COM server exports on Windows.
#[proc_macro_attribute]
pub fn handler(arguments: TokenStream, item: TokenStream) -> TokenStream {
    let parser = Punctuated::<syn::MetaNameValue, Token![,]>::parse_terminated;
    let arguments = match parser.parse(arguments) {
        Ok(arguments) => arguments,
        Err(error) => return error.into_compile_error().into(),
    };
    let mut clsid = None;
    for argument in arguments {
        if !argument.path.is_ident("clsid") {
            return syn::Error::new_spanned(argument.path, "unsupported handler argument")
                .into_compile_error()
                .into();
        }
        if clsid.is_some() {
            return syn::Error::new_spanned(argument.path, "duplicate clsid argument")
                .into_compile_error()
                .into();
        }
        let value = match argument.value {
            syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(value),
                ..
            }) => value,
            other => {
                return syn::Error::new_spanned(other, "clsid must be a string literal")
                    .into_compile_error()
                    .into();
            }
        };
        clsid = Some(value);
    }
    let Some(clsid) = clsid else {
        return syn::Error::new(proc_macro2::Span::call_site(), "expected clsid = \"...\"")
            .into_compile_error()
            .into();
    };
    let clsid_value = match parse_uuid(&clsid) {
        Ok(value) => value,
        Err(error) => return error.into_compile_error().into(),
    };
    let implementation = parse_macro_input!(item as ItemImpl);
    if implementation.trait_.is_none() {
        return syn::Error::new_spanned(
            &implementation.self_ty,
            "handler must mark a TaskHandler trait implementation",
        )
        .into_compile_error()
        .into();
    }
    if !implementation.generics.params.is_empty() {
        return syn::Error::new_spanned(
            &implementation.generics,
            "handler implementations must use a concrete, non-generic type",
        )
        .into_compile_error()
        .into();
    }
    let handler_type = &implementation.self_ty;
    quote! {
        #implementation

        #[doc(hidden)]
        pub const WINDOWS_TASK_HANDLER_CLSID: &str = #clsid;

        #[cfg(windows)]
        #[doc(hidden)]
        pub const WINDOWS_TASK_HANDLER_CLSID_GUID:
            ::windows_task::handler::__native::Guid =
            ::windows_task::handler::__native::Guid::from_u128(#clsid_value);

        #[cfg(windows)]
        #[unsafe(no_mangle)]
        #[allow(non_snake_case, reason = "COM requires this exact DLL export name")]
        pub extern "system" fn DllGetClassObject(
            requested: *const ::windows_task::handler::__native::Guid,
            iid: *const ::windows_task::handler::__native::Guid,
            output: *mut *mut ::core::ffi::c_void,
        ) -> ::windows_task::handler::__native::HResult {
            ::windows_task::handler::__native::dll_get_class_object::<#handler_type>(
                WINDOWS_TASK_HANDLER_CLSID_GUID,
                requested,
                iid,
                output,
            )
        }

        #[cfg(windows)]
        #[unsafe(no_mangle)]
        #[allow(non_snake_case, reason = "COM requires this exact DLL export name")]
        pub extern "system" fn DllCanUnloadNow()
            -> ::windows_task::handler::__native::HResult
        {
            ::windows_task::handler::__native::dll_can_unload_now()
        }
    }
    .into()
}

fn parse_uuid(value: &LitStr) -> syn::Result<u128> {
    let text = value.value();
    let bare = match (text.strip_prefix('{'), text.strip_suffix('}')) {
        (Some(without_open), Some(_)) => &without_open[..without_open.len() - 1],
        (None, None) => text.as_str(),
        _ => return Err(syn::Error::new(value.span(), "clsid has mismatched braces")),
    };
    let valid = if bare.len() == 36 {
        bare.char_indices().all(|(index, character)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                character == '-'
            } else {
                character.is_ascii_hexdigit()
            }
        })
    } else {
        bare.len() == 32 && bare.chars().all(|character| character.is_ascii_hexdigit())
    };
    if valid {
        let compact: String = bare.chars().filter(|character| *character != '-').collect();
        u128::from_str_radix(&compact, 16)
            .map_err(|_| syn::Error::new(value.span(), "clsid must be a UUID"))
    } else {
        Err(syn::Error::new(value.span(), "clsid must be a UUID"))
    }
}

#[cfg(test)]
mod tests {
    use proc_macro2::Span;
    use syn::LitStr;

    use super::parse_uuid;

    #[test]
    fn accepts_canonical_uuid_forms() {
        for value in [
            "e4ef9b55-4f33-4dd2-a658-6eb2c58c576b",
            "{e4ef9b55-4f33-4dd2-a658-6eb2c58c576b}",
            "e4ef9b554f334dd2a6586eb2c58c576b",
        ] {
            parse_uuid(&LitStr::new(value, Span::call_site()))
                .expect("canonical UUID should be accepted");
        }
    }

    #[test]
    fn rejects_malformed_uuid_forms() {
        for value in [
            "{e4ef9b55-4f33-4dd2-a658-6eb2c58c576b",
            "e4ef9b55-4f334-dd2-a658-6eb2c58c576b",
            "e4ef9b55_4f33_4dd2_a658_6eb2c58c576b",
        ] {
            parse_uuid(&LitStr::new(value, Span::call_site()))
                .expect_err("malformed UUID should be rejected");
        }
    }
}
