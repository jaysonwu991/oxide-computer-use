//! Accessibility (AX) element access: attributes, actions and tree snapshots.

use std::ffi::c_void;

use anyhow::{anyhow, bail, Result};
use core_foundation_sys::array::{CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef};
use core_foundation_sys::base::{kCFAllocatorDefault, CFTypeRef};
use core_foundation_sys::dictionary::CFDictionaryRef;
use core_foundation_sys::number::kCFBooleanTrue;

use super::cf;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CGPoint {
    pub x: f64,
    pub y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CGSize {
    pub width: f64,
    pub height: f64,
}

type AXUIElementRef = CFTypeRef;

const AX_VALUE_CG_POINT: u32 = 1;
const AX_VALUE_CG_SIZE: u32 = 2;
const AX_VALUE_CF_RANGE: u32 = 4;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CFRange {
    pub location: isize,
    pub length: isize,
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFTypeRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementCopyActionNames(element: AXUIElementRef, names: *mut CFArrayRef) -> i32;
    fn AXUIElementCopyParameterizedAttributeValue(
        element: AXUIElementRef,
        attribute: CFTypeRef,
        parameter: CFTypeRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementIsAttributeSettable(
        element: AXUIElementRef,
        attribute: CFTypeRef,
        settable: *mut bool,
    ) -> i32;
    fn AXValueCreate(the_type: u32, value: *const c_void) -> CFTypeRef;
    fn AXUIElementSetAttributeValue(
        element: AXUIElementRef,
        attribute: CFTypeRef,
        value: CFTypeRef,
    ) -> i32;
    fn AXUIElementPerformAction(element: AXUIElementRef, action: CFTypeRef) -> i32;
    fn AXValueGetValue(value: CFTypeRef, the_type: u32, out: *mut c_void) -> bool;
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
}

pub fn error_text(code: i32) -> String {
    let name = match code {
        -25200 => "kAXErrorFailure",
        -25201 => "kAXErrorIllegalArgument",
        -25202 => "kAXErrorInvalidUIElement",
        -25203 => "kAXErrorInvalidUIElementObserver",
        -25204 => "kAXErrorCannotComplete",
        -25205 => "kAXErrorAttributeUnsupported",
        -25206 => "kAXErrorActionUnsupported",
        -25207 => "kAXErrorNotificationUnsupported",
        -25208 => "kAXErrorNotImplemented",
        -25209 => "kAXErrorNotificationAlreadyRegistered",
        -25210 => "kAXErrorNotificationNotRegistered",
        -25211 => "kAXErrorAPIDisabled",
        -25212 => "kAXErrorNoValue",
        -25213 => "kAXErrorParameterizedAttributeUnsupported",
        -25214 => "kAXErrorNotEnoughPrecision",
        _ => "kAXErrorUnknown",
    };
    let hint = match code {
        -25204 => " (the app did not answer; it may be busy, modal, or unresponsive)",
        -25211 => " (Accessibility permission is missing for the calling terminal)",
        -25202 | -25212 => " (the element changed since the snapshot; call get_app_state again)",
        -25205 | -25206 => " (the target does not support this attribute or action)",
        _ => "",
    };
    format!("{name} ({code}){hint}")
}

fn check(code: i32, what: &str) -> Result<()> {
    if code == 0 {
        Ok(())
    } else {
        bail!("{what} failed: {}", error_text(code))
    }
}

pub fn trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

pub fn request_trust(prompt: bool) -> bool {
    unsafe {
        let Some(options) = cf::string("AXTrustedCheckOptionPrompt") else {
            return AXIsProcessTrusted();
        };
        let keys = [options.raw()];
        let values = [kCFBooleanTrue as CFTypeRef];
        let dict = core_foundation_sys::dictionary::CFDictionaryCreate(
            kCFAllocatorDefault,
            keys.as_ptr(),
            values.as_ptr(),
            1,
            &core_foundation_sys::dictionary::kCFTypeDictionaryKeyCallBacks,
            &core_foundation_sys::dictionary::kCFTypeDictionaryValueCallBacks,
        );
        if dict.is_null() {
            return AXIsProcessTrusted();
        }
        let trusted = AXIsProcessTrustedWithOptions(dict);
        core_foundation_sys::base::CFRelease(dict as CFTypeRef);
        let _ = prompt;
        trusted
    }
}

pub struct Element {
    raw: CFTypeRef,
}

impl Element {
    /// Retains a borrowed `AXUIElementRef`.
    pub unsafe fn from_get(raw: CFTypeRef) -> Option<Self> {
        cf::Owned::from_get(raw).map(Element::from_owned)
    }

    fn from_owned(owned: cf::Owned) -> Self {
        Element {
            raw: owned.into_raw(),
        }
    }

    pub fn retained(&self) -> Element {
        unsafe { Element::from_get(self.raw).expect("an accessible element is non-null") }
    }

    pub fn attribute(&self, name: &str) -> Option<cf::Owned> {
        unsafe {
            let key = cf::string(name)?;
            let mut value: CFTypeRef = std::ptr::null();
            let code = AXUIElementCopyAttributeValue(self.raw, key.raw(), &mut value);
            if code != 0 {
                return None;
            }
            cf::Owned::from_create(value)
        }
    }

    pub fn string_attribute(&self, name: &str) -> Option<String> {
        let value = self.attribute(name)?;
        unsafe { cf::string_from(value.raw()) }
    }

    pub fn bool_attribute(&self, name: &str) -> Option<bool> {
        let value = self.attribute(name)?;
        cf::boolean(value.raw())
    }

    pub fn elements_attribute(&self, name: &str) -> Vec<Element> {
        let Some(value) = self.attribute(name) else {
            return Vec::new();
        };
        unsafe {
            if cf::type_id(value.raw()) != core_foundation_sys::array::CFArrayGetTypeID() {
                return Vec::new();
            }
            let array = value.raw() as CFArrayRef;
            let count = CFArrayGetCount(array);
            (0..count)
                .filter_map(|index| {
                    let item = CFArrayGetValueAtIndex(array, index);
                    if item.is_null() {
                        None
                    } else {
                        Element::from_get(item as CFTypeRef)
                    }
                })
                .collect()
        }
    }

    pub fn role(&self) -> String {
        self.string_attribute("AXRole").unwrap_or_default()
    }

    pub fn actions(&self) -> Vec<String> {
        unsafe {
            let mut names: CFArrayRef = std::ptr::null();
            let code = AXUIElementCopyActionNames(self.raw, &mut names);
            if code != 0 || names.is_null() {
                return Vec::new();
            }
            let Some(owned) = cf::Owned::from_create(names as CFTypeRef) else {
                return Vec::new();
            };
            let array = owned.raw() as CFArrayRef;
            let count = CFArrayGetCount(array);
            (0..count)
                .filter_map(|index| {
                    let item = CFArrayGetValueAtIndex(array, index);
                    cf::string_from(item as CFTypeRef)
                })
                .collect()
        }
    }

    /// Whether writing `attribute` would be accepted; a settable element that
    /// ignores the write is still possible, which is why `set_value` reads back.
    pub fn is_settable(&self, attribute: &str) -> bool {
        unsafe {
            let Some(key) = cf::string(attribute) else {
                return false;
            };
            let mut settable = false;
            let code = AXUIElementIsAttributeSettable(self.raw, key.raw(), &mut settable);
            code == 0 && settable
        }
    }

    /// An attribute whose value is a single element, such as `AXParent`.
    pub fn element_attribute(&self, name: &str) -> Option<Element> {
        let value = self.attribute(name)?;
        unsafe { Element::from_get(value.raw()) }
    }

    /// The nearest ancestor whose role is `AXWindow`, which is what binds an
    /// element index to one window.
    pub fn window(&self) -> Option<Element> {
        let mut current = self.retained();
        for _ in 0..64 {
            if current.role() == "AXWindow" {
                return Some(current);
            }
            current = current.element_attribute("AXParent")?;
        }
        None
    }

    pub fn number_attribute(&self, name: &str) -> Option<f64> {
        self.attribute(name)
            .and_then(|value| cf::number(value.raw()))
    }

    pub fn character_count(&self) -> Option<usize> {
        self.number_attribute("AXNumberOfCharacters")
            .map(|count| count.max(0.0) as usize)
    }

    pub fn same_element(&self, other: &Element) -> bool {
        unsafe { core_foundation_sys::base::CFEqual(self.raw, other.raw) != 0 }
    }

    pub fn focused_element(&self) -> Option<Element> {
        self.element_attribute("AXFocusedUIElement")
    }

    fn parameterized(&self, attribute: &str, parameter: CFTypeRef) -> Option<cf::Owned> {
        unsafe {
            let key = cf::string(attribute)?;
            let mut value: CFTypeRef = std::ptr::null();
            let code = AXUIElementCopyParameterizedAttributeValue(
                self.raw,
                key.raw(),
                parameter,
                &mut value,
            );
            if code != 0 {
                return None;
            }
            cf::Owned::from_create(value)
        }
    }

    /// Reads a span of the element's text, for app text that exposes no
    /// `AXValue` string.
    pub fn string_for_range(&self, range: CFRange) -> Option<String> {
        unsafe {
            let value = AXValueCreate(AX_VALUE_CF_RANGE, &range as *const CFRange as *const c_void);
            let value = cf::Owned::from_create(value)?;
            let text = self.parameterized("AXStringForRange", value.raw())?;
            cf::string_from(text.raw())
        }
    }

    pub fn text(&self) -> Option<String> {
        self.string_value().or_else(|| {
            let count = self.character_count().filter(|count| *count > 0)?;
            self.string_for_range(CFRange {
                location: 0,
                length: count as isize,
            })
        })
    }

    /// Selects a character range, which is how `select_text` marks text.
    pub fn select_range(&self, range: CFRange) -> Result<()> {
        unsafe {
            let value = AXValueCreate(AX_VALUE_CF_RANGE, &range as *const CFRange as *const c_void);
            let Some(value) = cf::Owned::from_create(value) else {
                bail!("could not build an AX range value");
            };
            let key =
                cf::string("AXSelectedTextRange").ok_or_else(|| anyhow!("unknown attribute"))?;
            check(
                AXUIElementSetAttributeValue(self.raw, key.raw(), value.raw()),
                "AXSelectedTextRange",
            )
        }
    }

    pub fn selected_text(&self) -> Option<String> {
        self.string_attribute("AXSelectedText")
    }

    pub fn string_value(&self) -> Option<String> {
        self.string_attribute("AXValue")
    }

    pub fn perform(&self, action: &str) -> Result<()> {
        unsafe {
            let name = cf::string(action).ok_or_else(|| anyhow!("invalid action name"))?;
            check(AXUIElementPerformAction(self.raw, name.raw()), action)
        }
    }

    pub fn has_action(&self, action: &str) -> bool {
        self.actions().iter().any(|name| name == action)
    }

    pub fn set_string(&self, attribute: &str, value: &str) -> Result<()> {
        unsafe {
            let key = cf::string(attribute).ok_or_else(|| anyhow!("invalid attribute name"))?;
            let text = cf::string(value).ok_or_else(|| anyhow!("invalid value"))?;
            check(
                AXUIElementSetAttributeValue(self.raw, key.raw(), text.raw()),
                attribute,
            )
        }
    }

    pub fn focus(&self) -> Result<()> {
        let key = "AXFocused";
        unsafe {
            let Some(name) = cf::string(key) else {
                return Ok(());
            };
            let code =
                AXUIElementSetAttributeValue(self.raw, name.raw(), kCFBooleanTrue as CFTypeRef);
            if code != 0 {
                bail!("focusing the element failed: {}", error_text(code));
            }
            Ok(())
        }
    }

    pub fn activate_app(&self) -> Result<()> {
        unsafe {
            let Some(name) = cf::string("AXFrontmost") else {
                return Ok(());
            };
            let code =
                AXUIElementSetAttributeValue(self.raw, name.raw(), kCFBooleanTrue as CFTypeRef);
            if code != 0 {
                bail!("activating the app failed: {}", error_text(code));
            }
            Ok(())
        }
    }

    pub fn frame(&self) -> Option<(CGPoint, CGSize)> {
        let point = self.ax_value("AXPosition", AX_VALUE_CG_POINT)?;
        let size = self.ax_value("AXSize", AX_VALUE_CG_SIZE)?;
        Some((
            CGPoint {
                x: point[0],
                y: point[1],
            },
            CGSize {
                width: size[0],
                height: size[1],
            },
        ))
    }

    fn ax_value(&self, name: &str, kind: u32) -> Option<[f64; 2]> {
        let value = self.attribute(name)?;
        unsafe {
            let mut buffer = [0f64; 2];
            let ok = AXValueGetValue(value.raw(), kind, buffer.as_mut_ptr() as *mut c_void);
            ok.then_some(buffer)
        }
    }

    pub fn center(&self) -> Option<CGPoint> {
        let (point, size) = self.frame()?;
        if size.width <= 0.0 || size.height <= 0.0 {
            return None;
        }
        Some(CGPoint {
            x: point.x + size.width / 2.0,
            y: point.y + size.height / 2.0,
        })
    }
}

impl Clone for Element {
    fn clone(&self) -> Self {
        self.retained()
    }
}

impl Drop for Element {
    fn drop(&mut self) {
        unsafe { core_foundation_sys::base::CFRelease(self.raw) }
    }
}

pub fn application(pid: i32) -> Option<Element> {
    unsafe {
        let raw = AXUIElementCreateApplication(pid);
        cf::Owned::from_create(raw).map(Element::from_owned)
    }
}
