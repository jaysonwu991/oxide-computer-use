//! Minimal Core Foundation plumbing: owned handles, string and number bridging.

use core_foundation_sys::base::{kCFAllocatorDefault, CFGetTypeID, CFRelease, CFRetain, CFTypeRef};
use core_foundation_sys::number::{
    kCFNumberDoubleType, kCFNumberSInt64Type, CFNumberGetTypeID, CFNumberGetValue, CFNumberRef,
};
use core_foundation_sys::number::{CFBooleanGetTypeID, CFBooleanGetValue};
use core_foundation_sys::string::{
    kCFStringEncodingUTF8, CFStringCreateWithBytes, CFStringGetCString, CFStringGetLength,
    CFStringGetMaximumSizeForEncoding, CFStringGetTypeID, CFStringRef,
};

/// An owned CF object. Released on drop.
pub struct Owned(CFTypeRef);

impl Owned {
    /// Wraps a +1 reference produced by a `Create`/`Copy` function.
    pub unsafe fn from_create(raw: CFTypeRef) -> Option<Self> {
        if raw.is_null() {
            None
        } else {
            Some(Owned(raw))
        }
    }

    /// Wraps a borrowed reference and retains it.
    pub unsafe fn from_get(raw: CFTypeRef) -> Option<Self> {
        if raw.is_null() {
            None
        } else {
            Some(Owned(CFRetain(raw)))
        }
    }

    pub fn raw(&self) -> CFTypeRef {
        self.0
    }

    pub fn into_raw(self) -> CFTypeRef {
        let raw = self.0;
        std::mem::forget(self);
        raw
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) }
    }
}

pub unsafe fn string(value: &str) -> Option<Owned> {
    let raw = CFStringCreateWithBytes(
        kCFAllocatorDefault,
        value.as_ptr(),
        value.len() as isize,
        kCFStringEncodingUTF8,
        false as u8,
    );
    Owned::from_create(raw as CFTypeRef)
}

pub unsafe fn string_from(raw: CFTypeRef) -> Option<String> {
    if raw.is_null() || type_id(raw) != CFStringGetTypeID() {
        return None;
    }
    let text = raw as CFStringRef;
    let length = CFStringGetLength(text);
    let capacity = CFStringGetMaximumSizeForEncoding(length, kCFStringEncodingUTF8) + 1;
    if capacity <= 0 {
        return Some(String::new());
    }
    let mut buffer = vec![0u8; capacity as usize];
    let copied = CFStringGetCString(
        text,
        buffer.as_mut_ptr() as *mut i8,
        capacity,
        kCFStringEncodingUTF8,
    );
    if copied == 0 {
        return None;
    }
    let end = buffer.iter().position(|byte| *byte == 0).unwrap_or(0);
    buffer.truncate(end);
    String::from_utf8(buffer).ok()
}

pub fn type_id(raw: CFTypeRef) -> usize {
    unsafe { CFGetTypeID(raw) }
}

pub fn is_number(raw: CFTypeRef) -> bool {
    !raw.is_null() && type_id(raw) == unsafe { CFNumberGetTypeID() }
}

pub fn is_boolean(raw: CFTypeRef) -> bool {
    !raw.is_null() && type_id(raw) == unsafe { CFBooleanGetTypeID() }
}

pub fn number(raw: CFTypeRef) -> Option<f64> {
    if !is_number(raw) {
        return None;
    }
    let mut value: f64 = 0.0;
    let ok = unsafe {
        CFNumberGetValue(
            raw as CFNumberRef,
            kCFNumberDoubleType,
            &mut value as *mut f64 as *mut std::ffi::c_void,
        )
    };
    ok.then_some(value)
}

pub fn integer(raw: CFTypeRef) -> Option<i64> {
    if !is_number(raw) {
        return None;
    }
    let mut value: i64 = 0;
    let ok = unsafe {
        CFNumberGetValue(
            raw as CFNumberRef,
            kCFNumberSInt64Type,
            &mut value as *mut i64 as *mut std::ffi::c_void,
        )
    };
    ok.then_some(value)
}

pub fn boolean(raw: CFTypeRef) -> Option<bool> {
    is_boolean(raw).then(|| unsafe { CFBooleanGetValue(raw as _) })
}
