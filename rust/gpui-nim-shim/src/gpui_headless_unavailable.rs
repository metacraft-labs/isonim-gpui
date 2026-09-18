//! The feature-less arm of the six `gpui_headless` exports.
//!
//! ## Why this module exists
//!
//! `gpui_headless` is gated as a whole — `#[cfg(feature =
//! "gpui-headless")] pub mod gpui_headless;` in `lib.rs` — against a
//! crate whose `default = []`. So a plain `cargo build` (which is what
//! `just rust-build`, `repro.nim` and every consumer of
//! `rust/target/debug/libgpui_nim_shim.so` produce) shipped a cdylib
//! with **56** exported `gpui_*` symbols while `bindings.nim` declared
//! **62** under one `{.push dynlib.}`. Nim's `dynlib` import resolves
//! every referenced symbol at process start, so the first missing one
//! aborted the process before `main`:
//!
//! ```text
//! $ ./build/backends/isonim-examples-gpui --demo=tasks --port 39871
//! could not import: gpui_bump_generation
//! ```
//!
//! That killed four `isonim-examples` launcher gates
//! (`test_gpui_launcher_element_tree`, `test_cross_renderer_component_paths`,
//! `test_launcher_select_story_e2e`, and — via the sibling defect in
//! `isonim-freya` — `test_freya_launcher_element_tree`), all of which
//! reported the same distal symptom: "launcher … did not bind to
//! 127.0.0.1:<port> within 4s".
//!
//! ## Why a stub arm rather than turning the feature on
//!
//! `gpui-headless` pulls `gpui` + `gpui_platform` + `image` and the real
//! `HeadlessAppContext` machinery. The default, feature-less profile is
//! deliberately the *shadow-tree* shim — no GPU, no windowing, no
//! `gpui` dependency at all — and that is the profile every headless
//! element-tree test and every launcher on Linux is built against.
//! Making the feature non-optional would change what those tests test.
//!
//! So this module does what `gpui_launch` already does one layer up
//! (`lib.rs`, its `#[cfg(feature = "gpui-backend")]` /
//! `#[cfg(not(...))]` pair): it keeps the **ABI** constant across
//! feature selections and lets the *behaviour* vary. The symbol set a
//! consumer links against stops depending on which features the shim
//! happened to be built with, which is the actual defect — a binding
//! that exists in one build and not another is not a feature flag, it is
//! a load-time crash.
//!
//! ## What each function does without the feature
//!
//! Every one of them returns a value the FFI contract already defines
//! for "this cannot be served", so no caller needs a new branch:
//!
//! - `gpui_bump_generation` is **fully functional**. It is pure
//!   bookkeeping — a monotonic `AtomicU64` — and needs nothing from
//!   `gpui`. Callers bump it before mutating the tree; with no async
//!   render worker there is never a stale in-flight render to reject,
//!   but the counter still advances so the contract ("returns the new,
//!   post-bump generation") holds exactly.
//! - `gpui_render_to_pixels` returns `ErrorCode::RendererUnavailable`
//!   (2) and writes `(null, 0)`. That is the same code the real arm
//!   returns on a platform with no headless renderer, and `bindings.nim`
//!   already documents it.
//! - `gpui_render_submit_async` returns `0`, which its own doc comment
//!   defines as "the worker thread is not running … callers should fall
//!   back to the synchronous path".
//! - `gpui_render_try_take` returns `TAKE_UNKNOWN_TOKEN` (-100), which
//!   is correct rather than merely convenient: no token was ever issued.
//! - `gpui_render_cancel` and `gpui_free_pixels` are no-ops, because
//!   this arm never allocates and never issues a token.
//!
//! ## The gate that now catches this class
//!
//! `tools/check_bindings.sh` compared `extern "C"` declarations read out
//! of the *sources* — `cfg`-gated ones included — against `bindings.nim`,
//! so it reported 62 == 62 and passed on a build that exported 56.
//! `tools/check_exported_symbols.sh` compares `bindings.nim` against
//! `nm -D` of a **built artifact** instead. See
//! `codetracer-specs/Testing/Verification-Harness-Traps.md` §18:
//! diagnose the class in the source, enumerate it in the linked image.

use std::sync::atomic::{AtomicU64, Ordering};

/// Mirror of `gpui_headless::ErrorCode::RendererUnavailable`. Spelled as
/// a literal here rather than shared with the real arm because that
/// enum lives inside the gated module; the numeric values are the
/// stable part of the contract and are documented in `bindings.nim`.
const ERROR_RENDERER_UNAVAILABLE: i32 = 2;
const ERROR_INVALID_ARGS: i32 = 1;

/// Mirror of `gpui_headless::TAKE_UNKNOWN_TOKEN`.
const TAKE_UNKNOWN_TOKEN: i32 = -100;

/// The story / tree generation counter. Process-wide, exactly as in the
/// real arm — the FFI surface has no handle parameter.
static CURRENT_GENERATION: AtomicU64 = AtomicU64::new(0);

/// See the module docs. Fully functional without `gpui`.
#[no_mangle]
pub extern "C" fn gpui_bump_generation() -> u64 {
    // `fetch_add` returns the prior value; add one to obtain the
    // post-bump generation, matching the real arm's return contract.
    CURRENT_GENERATION.fetch_add(1, Ordering::AcqRel) + 1
}

/// # Safety
///
/// `out_ptr` and `out_len` MUST be non-null and point to writable
/// storage, as in the real arm.
#[no_mangle]
pub extern "C" fn gpui_render_to_pixels(
    _width: u32,
    _height: u32,
    _scale: f32,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    if out_ptr.is_null() || out_len.is_null() {
        return ERROR_INVALID_ARGS;
    }
    // Initialise the out parameters so a caller that always frees stays
    // safe, exactly as the real arm does before its own early returns.
    unsafe {
        *out_ptr = std::ptr::null_mut();
        *out_len = 0;
    }
    ERROR_RENDERER_UNAVAILABLE
}

/// # Safety
///
/// This arm never allocates, so the only correct argument is a null
/// pointer / zero length. Signature and safety contract are kept
/// identical to the real arm so the ABI does not vary with features.
#[no_mangle]
pub unsafe extern "C" fn gpui_free_pixels(_ptr: *mut u8, _len: usize) {}

#[no_mangle]
pub extern "C" fn gpui_render_submit_async(_width: u32, _height: u32, _scale: f32) -> u32 {
    // 0 == "the worker thread is not running"; the documented signal to
    // fall back to the synchronous path.
    0
}

/// # Safety
///
/// `out_ptr` and `out_len` MUST be non-null and point to writable
/// storage, as in the real arm.
#[no_mangle]
pub extern "C" fn gpui_render_try_take(
    _token: u32,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    if out_ptr.is_null() || out_len.is_null() {
        return -ERROR_INVALID_ARGS;
    }
    unsafe {
        *out_ptr = std::ptr::null_mut();
        *out_len = 0;
    }
    // No token was ever issued by `gpui_render_submit_async` on this
    // arm, so every token is unknown. This is the accurate answer, not
    // a convenient one.
    TAKE_UNKNOWN_TOKEN
}

#[no_mangle]
pub extern "C" fn gpui_render_cancel(_token: u32) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bump_generation_is_monotonic_without_the_feature() {
        let a = gpui_bump_generation();
        let b = gpui_bump_generation();
        assert!(b > a, "generation must advance: {a} -> {b}");
        assert_eq!(b, a + 1);
    }

    #[test]
    fn render_to_pixels_reports_renderer_unavailable_and_nulls_the_out_params() {
        let mut ptr: *mut u8 = 0x1 as *mut u8;
        let mut len: usize = 99;
        let rc = gpui_render_to_pixels(10, 10, 1.0, &mut ptr, &mut len);
        assert_eq!(rc, ERROR_RENDERER_UNAVAILABLE);
        assert!(ptr.is_null());
        assert_eq!(len, 0);
    }

    #[test]
    fn render_to_pixels_rejects_null_out_params() {
        let rc = gpui_render_to_pixels(10, 10, 1.0, std::ptr::null_mut(), std::ptr::null_mut());
        assert_eq!(rc, ERROR_INVALID_ARGS);
    }

    #[test]
    fn submit_async_reports_no_worker() {
        assert_eq!(gpui_render_submit_async(10, 10, 1.0), 0);
    }

    #[test]
    fn try_take_reports_unknown_token() {
        let mut ptr: *mut u8 = 0x1 as *mut u8;
        let mut len: usize = 99;
        let rc = gpui_render_try_take(7, &mut ptr, &mut len);
        assert_eq!(rc, TAKE_UNKNOWN_TOKEN);
        assert!(ptr.is_null());
        assert_eq!(len, 0);
    }
}
