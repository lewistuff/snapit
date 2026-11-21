//! System-wide hotkeys through Carbon's RegisterEventHotKey: the only public
//! API that consumes the keystroke without needing Accessibility permission.
//! The macOS 27 SDK headers no longer declare these functions, but
//! HIToolbox still exports them, so the prototypes are declared here (research R2).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr;

#[repr(C)]
struct EventTypeSpec {
    class: u32,
    kind: u32,
}

#[repr(C)]
#[derive(Default)]
struct EventHotKeyID {
    signature: u32,
    id: u32,
}

type EventHandler = extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn GetApplicationEventTarget() -> *mut c_void;
    fn InstallEventHandler(
        target: *mut c_void,
        handler: EventHandler,
        count: usize,
        types: *const EventTypeSpec,
        user_data: *mut c_void,
        out_ref: *mut *mut c_void,
    ) -> i32;
    fn RegisterEventHotKey(
        code: u32,
        modifiers: u32,
        id: EventHotKeyID,
        target: *mut c_void,
        options: u32,
        out_ref: *mut *mut c_void,
    ) -> i32;
    fn UnregisterEventHotKey(hotkey: *mut c_void) -> i32;
    fn GetEventParameter(
        event: *mut c_void,
        name: u32,
        desired_type: u32,
        actual_type: *mut u32,
        size: usize,
        actual_size: *mut usize,
        data: *mut c_void,
    ) -> i32;
}

const fn four_cc(s: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*s)
}

const SIGNATURE: u32 = four_cc(b"snap");
const K_EVENT_CLASS_KEYBOARD: u32 = four_cc(b"keyb");
const K_EVENT_HOT_KEY_PRESSED: u32 = 5;
const K_EVENT_PARAM_DIRECT_OBJECT: u32 = four_cc(b"----");
const TYPE_EVENT_HOT_KEY_ID: u32 = four_cc(b"hkid");
/// Returned when another process already registered the same combination.
pub const ERR_EXISTS: i32 = -9878;

thread_local! {
    static CALLBACK: Cell<Option<fn(u32)>> = const { Cell::new(None) };
    static REFS: RefCell<HashMap<u32, *mut c_void>> = RefCell::new(HashMap::new());
}

extern "C" fn on_event(_: *mut c_void, event: *mut c_void, _: *mut c_void) -> i32 {
    let mut id = EventHotKeyID::default();
    // SAFETY: the out buffer is a correctly sized EventHotKeyID.
    let status = unsafe {
        GetEventParameter(
            event,
            K_EVENT_PARAM_DIRECT_OBJECT,
            TYPE_EVENT_HOT_KEY_ID,
            ptr::null_mut(),
            size_of::<EventHotKeyID>(),
            ptr::null_mut(),
            (&raw mut id).cast(),
        )
    };
    if status == 0
        && id.signature == SIGNATURE
        && let Some(cb) = CALLBACK.get()
    {
        cb(id.id);
    }
    0
}

/// Installs the handler that receives every hotkey press. Call once, on the main thread.
pub fn install(callback: fn(u32)) {
    CALLBACK.set(Some(callback));
    let spec = EventTypeSpec { class: K_EVENT_CLASS_KEYBOARD, kind: K_EVENT_HOT_KEY_PRESSED };
    // SAFETY: plain C call with valid pointers; the handler is a 'static fn.
    unsafe {
        InstallEventHandler(GetApplicationEventTarget(), on_event, 1, &spec, ptr::null_mut(), ptr::null_mut());
    }
}

/// Registers `id` for the combination, replacing any earlier registration of `id`.
pub fn register(id: u32, code: u32, modifiers: u32) -> Result<(), i32> {
    unregister(id);
    let mut out = ptr::null_mut();
    let hk = EventHotKeyID { signature: SIGNATURE, id };
    // SAFETY: plain C call; `out` receives an opaque ref we later unregister.
    let status = unsafe { RegisterEventHotKey(code, modifiers, hk, GetApplicationEventTarget(), 0, &mut out) };
    if status != 0 {
        return Err(status);
    }
    REFS.with_borrow_mut(|r| r.insert(id, out));
    Ok(())
}

pub fn unregister(id: u32) {
    if let Some(r) = REFS.with_borrow_mut(|r| r.remove(&id)) {
        // SAFETY: `r` came from RegisterEventHotKey and is unregistered once.
        unsafe { UnregisterEventHotKey(r) };
    }
}

pub fn unregister_all() {
    let ids: Vec<u32> = REFS.with_borrow(|r| r.keys().copied().collect());
    ids.into_iter().for_each(unregister);
}
