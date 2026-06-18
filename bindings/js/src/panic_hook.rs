// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2018-2021 Rustwasm contributors
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! This is copied from the abandoned https://github.com/rustwasm/console_error_panic_hook
//! with minor edits and simplifications.

//! # `console_error_panic_hook`
//!
//! This module lets you debug panics on `wasm32-unknown-unknown` by providing a
//! panic hook that forwards panic messages to
//! [`console.error`](https://developer.mozilla.org/en-US/docs/Web/API/Console/error).
//!
//! When an error is reported with `console.error`, browser devtools and node.js
//! will typically capture a stack trace and display it with the logged error
//! message.
//!
//! Without `console_error_panic_hook` you just get something like *RuntimeError: Unreachable executed*
//!
//! With this panic hook installed you will see the full panic message that the
//! Rust side produced.
//!
//! ## Usage
//!
//! There are two ways to install this panic hook.
//!
//! First, you can set the hook yourself by calling `std::panic::set_hook` in
//! some initialization function:
//!
//! ```ignore
//! use std::panic;
//!
//! fn my_init_function() {
//!     panic::set_hook(Box::new(panic_hook::hook));
//!
//!     // ...
//! }
//! ```
//!
//! Alternatively, use `set_once` on some common code path to ensure that
//! `set_hook` is called, but only the one time. Under the hood, this uses
//! `std::sync::Once`.
//!
//! ```ignore
//! struct MyBigThing;
//!
//! impl MyBigThing {
//!     pub fn new() -> MyBigThing {
//!         panic_hook::set_once();
//!
//!         MyBigThing
//!     }
//! }
//! ```
//!
//! ## Error.stackTraceLimit
//!
//! Many browsers only capture the top 10 frames of a stack trace. In rust programs this is less likely to be enough. To see more frames, you can set the non-standard value `Error.stackTraceLimit`. For more information see the [MDN Web Docs](https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/Error/stackTraceLimit) or [v8 docs](https://v8.dev/docs/stack-trace-api).
//!

use std::panic;

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn error(msg: String);

    type Error;

    #[wasm_bindgen(constructor)]
    fn new() -> Error;

    #[wasm_bindgen(structural, method, getter)]
    fn stack(error: &Error) -> String;
}

/// A panic hook for use with
/// [`std::panic::set_hook`](https://doc.rust-lang.org/nightly/std/panic/fn.set_hook.html)
/// that logs panics into
/// [`console.error`](https://developer.mozilla.org/en-US/docs/Web/API/Console/error).
pub fn hook(info: &panic::PanicHookInfo) {
    let mut msg = info.to_string();

    // Add the error stack to our message.
    //
    // This ensures that even if the `console` implementation doesn't
    // include stacks for `console.error`, the stack is still available
    // for the user. Additionally, Firefox's console tries to clean up
    // stack traces, and ruins Rust symbols in the process
    // (https://bugzilla.mozilla.org/show_bug.cgi?id=1519569) but since
    // it only touches the logged message's associated stack, and not
    // the message's contents, by including the stack in the message
    // contents we make sure it is available to the user.
    msg.push_str("\n\nStack:\n\n");
    let e = Error::new();
    let stack = e.stack();
    msg.push_str(&stack);

    // Safari's devtools, on the other hand, _do_ mess with logged
    // messages' contents, so we attempt to break their heuristics for
    // doing that by appending some whitespace.
    // https://github.com/rustwasm/console_error_panic_hook/issues/7
    msg.push_str("\n\n");

    // Finally, log the panic with `console.error`!
    error(msg);
}

/// Set the `console.error` panic hook the first time this is called. Subsequent
/// invocations do nothing.
#[inline]
pub fn set_once() {
    use std::sync::Once;
    static SET_HOOK: Once = Once::new();
    SET_HOOK.call_once(|| {
        panic::set_hook(Box::new(hook));
    });
}
