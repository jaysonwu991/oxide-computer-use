//! Clipboard-backed paste with restoration of the person's previous contents.

use std::ffi::{c_char, c_void};

use anyhow::{bail, Result};

type Object = *mut c_void;
type Sel = *const c_void;

#[link(name = "AppKit", kind = "framework")]
extern "C" {
    fn objc_msgSend();
    static NSPasteboardTypeString: Object;
    static NSPasteboardTypeHTML: Object;
}

#[link(name = "objc")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> Object;
    fn sel_registerName(name: *const c_char) -> Sel;
}

fn selector(name: &str) -> Sel {
    let name = std::ffi::CString::new(name).expect("a selector has no NUL");
    unsafe { sel_registerName(name.as_ptr()) }
}

fn class(name: &str) -> Result<Object> {
    let class_name = std::ffi::CString::new(name).expect("a class name has no NUL");
    let value = unsafe { objc_getClass(class_name.as_ptr()) };
    if value.is_null() {
        bail!("AppKit does not provide the {name} class");
    }
    Ok(value)
}

unsafe fn send0<R>(receiver: Object, name: &str) -> R {
    let call: unsafe extern "C" fn(Object, Sel) -> R =
        std::mem::transmute(objc_msgSend as *const ());
    call(receiver, selector(name))
}

unsafe fn send1<A, R>(receiver: Object, name: &str, arg: A) -> R {
    let call: unsafe extern "C" fn(Object, Sel, A) -> R =
        std::mem::transmute(objc_msgSend as *const ());
    call(receiver, selector(name), arg)
}

unsafe fn send2<A, B, R>(receiver: Object, name: &str, first: A, second: B) -> R {
    let call: unsafe extern "C" fn(Object, Sel, A, B) -> R =
        std::mem::transmute(objc_msgSend as *const ());
    call(receiver, selector(name), first, second)
}

unsafe fn string(value: &str) -> Result<Object> {
    let value = std::ffi::CString::new(value)?;
    let cls = class("NSString")?;
    let text: Object = send1(cls, "stringWithUTF8String:", value.as_ptr());
    if text.is_null() {
        bail!("could not create a pasteboard string");
    }
    Ok(text)
}

unsafe fn new_pasteboard_item() -> Result<Object> {
    let cls = class("NSPasteboardItem")?;
    let allocated: Object = send0(cls, "alloc");
    let item: Object = send0(allocated, "init");
    if item.is_null() {
        bail!("could not create a pasteboard item");
    }
    Ok(item)
}

struct Pool(Object);

impl Pool {
    fn new() -> Result<Pool> {
        unsafe {
            let cls = class("NSAutoreleasePool")?;
            let allocated: Object = send0(cls, "alloc");
            let pool: Object = send0(allocated, "init");
            if pool.is_null() {
                bail!("could not create an autorelease pool");
            }
            Ok(Pool(pool))
        }
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        unsafe {
            let _: () = send0(self.0, "drain");
        }
    }
}

unsafe fn clone_items(pasteboard: Object) -> Result<Object> {
    let old: Object = send0(pasteboard, "pasteboardItems");
    if old.is_null() {
        return Ok(std::ptr::null_mut());
    }
    let count: usize = send0(old, "count");
    let array_class = class("NSMutableArray")?;
    let copied: Object = send1(array_class, "arrayWithCapacity:", count);
    if copied.is_null() {
        bail!("could not snapshot the clipboard");
    }

    for index in 0..count {
        let item: Object = send1(old, "objectAtIndex:", index);
        let types: Object = send0(item, "types");
        if types.is_null() {
            continue;
        }
        let copied_item = new_pasteboard_item()?;
        let type_count: usize = send0(types, "count");
        for type_index in 0..type_count {
            let kind: Object = send1(types, "objectAtIndex:", type_index);
            let data: Object = send1(item, "dataForType:", kind);
            if !data.is_null() {
                let _: i8 = send2(copied_item, "setData:forType:", data, kind);
            }
        }
        let _: () = send1(copied, "addObject:", copied_item);
        let _: () = send0(copied_item, "release");
    }
    Ok(copied)
}

unsafe fn restore(pasteboard: Object, previous: Object) -> Result<()> {
    let _: isize = send0(pasteboard, "clearContents");
    if previous.is_null() {
        return Ok(());
    }
    let ok: i8 = send1(pasteboard, "writeObjects:", previous);
    if ok == 0 {
        bail!("the previous clipboard contents could not be restored");
    }
    Ok(())
}

unsafe fn write_text(pasteboard: Object, text: &str, format: &str) -> Result<()> {
    let item = new_pasteboard_item()?;
    let value = string(text)?;
    let kind = match format {
        "html" => NSPasteboardTypeHTML,
        "text" | "md" => NSPasteboardTypeString,
        other => {
            let _: () = send0(item, "release");
            bail!("unknown paste format `{other}`; use text, md or html");
        }
    };
    let ok: i8 = send2(item, "setString:forType:", value, kind);
    if ok == 0 {
        let _: () = send0(item, "release");
        bail!("the pasteboard rejected the {format} content");
    }
    let array_class = class("NSArray")?;
    let array: Object = send1(array_class, "arrayWithObject:", item);
    let cleared: isize = send0(pasteboard, "clearContents");
    if cleared == 0 {
        let _: () = send0(item, "release");
        bail!("could not take ownership of the clipboard");
    }
    let written: i8 = send1(pasteboard, "writeObjects:", array);
    let _: () = send0(item, "release");
    if written == 0 {
        bail!("the pasteboard rejected the new content");
    }
    Ok(())
}

/// Put `text` on the general pasteboard, run the paste gesture, then restore
/// every data type from the previous clipboard items.
pub fn paste_with_restore(
    text: &str,
    format: &str,
    send_paste: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let _pool = Pool::new()?;
    unsafe {
        let cls = class("NSPasteboard")?;
        let pasteboard: Object = send0(cls, "generalPasteboard");
        if pasteboard.is_null() {
            bail!("the system has no general pasteboard");
        }
        let previous = clone_items(pasteboard)?;
        let result = write_text(pasteboard, text, format).and_then(|()| send_paste());
        let restored = restore(pasteboard, previous);
        match (result, restored) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) => Err(error),
            (Ok(()), Err(error)) => Err(error),
            (Err(error), Err(restore_error)) => {
                bail!("{error:#}; additionally, the clipboard restore failed: {restore_error:#}")
            }
        }
    }
}
