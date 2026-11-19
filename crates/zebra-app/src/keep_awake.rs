//! The Mac stays awake while a show runs: no display sleep and no screen
//! saver mid-talk. An IOKit power assertion
//! (`kIOPMAssertionTypePreventUserIdleDisplaySleep`) is held while any
//! window presents (presenter view included) and released when the show
//! ends, its window closes or Zebra quits. `pmset -g assertions` lists it.
//!
//! The assertion is an RAII guard: dropping it releases it. Off macOS
//! nothing is held, but the state is kept the same way so it can be tested.

/// Something that can keep the machine awake. `hold` returns a guard that
/// lets go when dropped, or `None` if the system refused.
pub trait Backend {
    type Guard;
    fn hold(&mut self, reason: &str) -> Option<Self::Guard>;
}

/// Holds a guard exactly while [`KeepAwake::set`] is told the app presents.
pub struct KeepAwake<B: Backend> {
    backend: B,
    guard: Option<B::Guard>,
    /// What was last asked for, so a refused hold isn't retried each frame.
    wanted: bool,
}

impl<B: Backend> KeepAwake<B> {
    pub fn new(backend: B) -> Self {
        KeepAwake { backend, guard: None, wanted: false }
    }

    /// Call every frame with whether any window is presenting. `reason`
    /// is only built when a hold starts.
    pub fn set(&mut self, presenting: bool, reason: impl FnOnce() -> String) {
        if presenting == self.wanted {
            return;
        }
        self.wanted = presenting;
        self.guard = if presenting { self.backend.hold(&reason()) } else { None };
    }

    pub fn is_held(&self) -> bool {
        self.guard.is_some()
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }
}

/// The real thing: an IOKit assertion on macOS, nothing elsewhere.
pub struct System;

pub type SystemKeepAwake = KeepAwake<System>;

impl Default for SystemKeepAwake {
    fn default() -> Self {
        KeepAwake::new(System)
    }
}

#[cfg(target_os = "macos")]
impl Backend for System {
    type Guard = iokit::Assertion;
    fn hold(&mut self, reason: &str) -> Option<iokit::Assertion> {
        iokit::Assertion::prevent_display_sleep(reason)
    }
}

/// Off macOS a show holds a guard that does nothing, so the shell's state
/// can be checked on Linux.
#[cfg(not(target_os = "macos"))]
impl Backend for System {
    type Guard = ();
    fn hold(&mut self, _reason: &str) -> Option<()> {
        Some(())
    }
}

#[cfg(target_os = "macos")]
mod iokit {
    use std::ffi::c_void;

    use objc2::rc::Retained;
    use objc2_foundation::NSString;

    /// `kIOPMAssertionLevelOn`.
    const LEVEL_ON: u32 = 255;
    /// `kIOReturnSuccess`.
    const SUCCESS: i32 = 0;

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        /// The CFStrings are passed as NSString (toll-free bridged).
        fn IOPMAssertionCreateWithName(assertion_type: *const c_void, level: u32, name: *const c_void, id: *mut u32) -> i32;
        fn IOPMAssertionRelease(id: u32) -> i32;
    }

    /// A held power assertion; released on drop.
    pub struct Assertion(u32);

    impl Assertion {
        pub fn prevent_display_sleep(reason: &str) -> Option<Assertion> {
            // kIOPMAssertionTypePreventUserIdleDisplaySleep
            let kind = NSString::from_str("PreventUserIdleDisplaySleep");
            let name = NSString::from_str(reason);
            let mut id = 0u32;
            // SAFETY: both strings live across the call; `id` is a valid
            // out pointer.
            let r = unsafe { IOPMAssertionCreateWithName(Retained::as_ptr(&kind).cast(), LEVEL_ON, Retained::as_ptr(&name).cast(), &mut id) };
            if r != SUCCESS {
                eprintln!("zebra: could not keep the display awake (IOKit error {r:#x})");
                return None;
            }
            Some(Assertion(id))
        }
    }

    impl Drop for Assertion {
        fn drop(&mut self) {
            // SAFETY: the id came from IOPMAssertionCreateWithName and is
            // released once.
            unsafe { IOPMAssertionRelease(self.0) };
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;

    /// Counts live guards; can be told to refuse.
    #[derive(Default)]
    struct Fake {
        live: Rc<Cell<i32>>,
        holds: u32,
        refuse: bool,
        reason: String,
    }
    struct FakeGuard(Rc<Cell<i32>>);
    impl Drop for FakeGuard {
        fn drop(&mut self) {
            self.0.set(self.0.get() - 1);
        }
    }
    impl Backend for Fake {
        type Guard = FakeGuard;
        fn hold(&mut self, reason: &str) -> Option<FakeGuard> {
            self.holds += 1;
            self.reason = reason.to_string();
            if self.refuse {
                return None;
            }
            self.live.set(self.live.get() + 1);
            Some(FakeGuard(self.live.clone()))
        }
    }

    #[test]
    fn held_exactly_while_presenting() {
        let live = Rc::new(Cell::new(0));
        let mut k = KeepAwake::new(Fake { live: live.clone(), ..Default::default() });
        k.set(false, || unreachable!("not presenting: no reason needed"));
        assert_eq!(live.get(), 0);
        k.set(true, || "Presenting".into());
        assert!(k.is_held());
        assert_eq!(live.get(), 1);
        assert_eq!(k.backend().reason, "Presenting");
        // Frame after frame of the show: still the one assertion.
        for _ in 0..5 {
            k.set(true, || unreachable!("already held"));
        }
        assert_eq!((live.get(), k.backend().holds), (1, 1));
        k.set(false, String::new);
        assert!(!k.is_held());
        assert_eq!(live.get(), 0, "released when the show ends");
        // A second show takes it again.
        k.set(true, || "Presenting".into());
        assert_eq!((live.get(), k.backend().holds), (1, 2));
        drop(k);
        assert_eq!(live.get(), 0, "released when Zebra quits");
    }

    #[test]
    fn a_refused_hold_is_not_retried_every_frame() {
        let mut k = KeepAwake::new(Fake { refuse: true, ..Default::default() });
        for _ in 0..5 {
            k.set(true, || "Presenting".into());
        }
        assert!(!k.is_held());
        assert_eq!(k.backend().holds, 1);
    }
}
