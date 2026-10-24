//! macOS: Text Input Source Services (upstream's `TerminalInputSource`), the stored id the input source id
//! (`com.apple.keylayout.US`). The selectable layouts are the enabled keyboard input sources macOS allows selecting
//! (`TISCreateInputSourceList` over the keyboard category, `kTISPropertyInputSourceIsSelectCapable`); the current one
//! is `TISCopyCurrentKeyboardInputSource`'s; `TISSelectInputSource` selects, system-wide.

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2_core_foundation::{CFArray, CFBoolean, CFDictionary, CFRetained, CFString, CFType};

use super::{Backend, Failure, KeyboardLayout};

/// A `TISInputSourceRef`.
type InputSourceRef = *const c_void;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    static kTISPropertyInputSourceCategory: &'static CFString;
    static kTISCategoryKeyboardInputSource: &'static CFString;
    static kTISPropertyInputSourceIsSelectCapable: &'static CFString;
    static kTISPropertyInputSourceID: &'static CFString;
    static kTISPropertyLocalizedName: &'static CFString;
    fn TISCreateInputSourceList(properties: &CFDictionary, include_all_installed: u8) -> Option<NonNull<CFArray>>;
    fn TISCopyCurrentKeyboardInputSource() -> InputSourceRef;
    fn TISGetInputSourceProperty(source: InputSourceRef, key: &CFString) -> *const c_void;
    fn TISSelectInputSource(source: InputSourceRef) -> i32;
}

/// The input sources of Text Input Source Services.
pub struct InputSources;

/// The enabled input sources whose `key` property is `value`.
fn sources(key: &CFString, value: &CFType) -> Option<CFRetained<CFArray>> {
    let filter = CFDictionary::<CFString, CFType>::from_slices(&[key], &[value]);
    // SAFETY: the filter is a live dictionary of CF keys and values; the list comes back retained (a Create call).
    let list = unsafe { TISCreateInputSourceList(filter.as_opaque(), 0) }?;
    // SAFETY: a Create call's result is owned by the caller.
    Some(unsafe { CFRetained::from_raw(list) })
}

/// The input sources in `list`, unretained: they live as long as the list.
fn each(list: &CFArray) -> impl Iterator<Item = InputSourceRef> + '_ {
    (0..list.count()).map(move |index| {
        // SAFETY: `index` is within the array's count; its values are input sources.
        unsafe { list.value_at_index(index) }
    })
}

/// `source`'s string property `key`.
fn string(source: InputSourceRef, key: &CFString) -> Option<String> {
    // SAFETY: `source` is a live input source; the property is a CFString it owns, or null.
    let value = unsafe { TISGetInputSourceProperty(source, key) };
    // SAFETY: a non-null string property is a live CFString for as long as its source.
    let value = unsafe { value.cast::<CFString>().as_ref() }?;
    Some(value.to_string())
}

/// `source`'s boolean property `key`, false when unset.
fn flag(source: InputSourceRef, key: &CFString) -> bool {
    // SAFETY: `source` is a live input source; the property is a CFBoolean it owns, or null.
    let value = unsafe { TISGetInputSourceProperty(source, key) };
    // SAFETY: a non-null boolean property is a live CFBoolean for as long as its source.
    unsafe { value.cast::<CFBoolean>().as_ref() }.is_some_and(CFBoolean::as_bool)
}

/// The enabled keyboard input source with `id`, as a list holding it.
fn source_with_id(id: &str) -> Option<CFRetained<CFArray>> {
    let id = CFString::from_str(id);
    // SAFETY: a constant of the framework, valid for the process's life.
    let key = unsafe { kTISPropertyInputSourceID };
    sources(key, &id).filter(|list| list.count() > 0)
}

impl Backend for InputSources {
    fn layouts(&mut self) -> Result<Vec<KeyboardLayout>, Failure> {
        // SAFETY: constants of the framework, valid for the process's life.
        let (category, keyboard, selectable, id_key, name_key) = unsafe {
            (
                kTISPropertyInputSourceCategory,
                kTISCategoryKeyboardInputSource,
                kTISPropertyInputSourceIsSelectCapable,
                kTISPropertyInputSourceID,
                kTISPropertyLocalizedName,
            )
        };
        let list = sources(category, keyboard).ok_or("no input source list")?;
        Ok(each(&list)
            .filter(|source| flag(*source, selectable))
            .filter_map(|source| {
                let id = string(source, id_key)?;
                let name = string(source, name_key).unwrap_or_else(|| id.clone());
                Some(KeyboardLayout { id, name })
            })
            .collect())
    }

    fn current(&mut self) -> Result<String, Failure> {
        // SAFETY: a Copy call: the source comes back retained, or null.
        let source = unsafe { TISCopyCurrentKeyboardInputSource() };
        let source = NonNull::new(source.cast_mut().cast::<CFType>()).ok_or("no current input source")?;
        // SAFETY: a Copy call's result is owned by the caller, released when this goes.
        let source = unsafe { CFRetained::from_raw(source) };
        // SAFETY: a constant of the framework, valid for the process's life.
        let key = unsafe { kTISPropertyInputSourceID };
        string(CFRetained::as_ptr(&source).as_ptr().cast_const().cast(), key).ok_or_else(|| "no input source id".into())
    }

    fn select(&mut self, id: &str) -> Result<(), Failure> {
        let list = source_with_id(id).ok_or_else(|| format!("no input source {id}"))?;
        let source = each(&list).next().ok_or_else(|| format!("no input source {id}"))?;
        // SAFETY: `source` lives as long as `list`, held here.
        let status = unsafe { TISSelectInputSource(source) };
        if status == 0 {
            Ok(())
        } else {
            Err(format!("TISSelectInputSource({id}): {status}").into())
        }
    }
}
