#![cfg_attr(not(feature = "std"), no_std)]
#![allow(non_camel_case_types)]
//! Clean-room, header-free minimal ABI for VST3 hosting & modules.
//! This crate intentionally mirrors public documentation naming to preserve ABI.
//! It does **not** include or depend on Steinberg headers.
//!
//! Phase 1: FUnknown, IPluginFactory, GetPluginFactory.
//! Later phases: IPluginBase/IComponent/IAudioProcessor/etc.

#[cfg(feature = "std")]
extern crate std as alloc_std;

use core::ffi::c_void;
use core::ptr::NonNull;

// ----- Core scalar types (per public docs) -----------------------------------
pub type int16 = i16;
pub type int32 = i32;
pub type int64 = i64;
pub type uint32 = u32;
pub type uint64 = u64;
pub type tresult = int32;

// Success / failure codes (matching public semantics; numeric values chosen to
// match common usage in examples: 0 = OK, non-zero = failure; kNoInterface is
// commonly observed as a negative error code in host examples).
pub const K_RESULT_OK: tresult = 0;
pub const K_RESULT_FALSE: tresult = 1; // generic "false but not error"
pub const K_NOT_IMPLEMENTED: tresult = -1;
pub const K_NO_INTERFACE: tresult = -2;
pub const K_INVALID_ARG: tresult = -3;
pub const K_INTERNAL_ERR: tresult = -4;

/// 16-byte type used for IIDs/CIDs (a.k.a. TUID in docs).
#[repr(C)]
#[derive(Copy, Clone, Eq, PartialEq)]
pub struct Tuid(pub [u8; 16]);

impl Tuid {
    pub const fn new(bytes: [u8; 16]) -> Self { Self(bytes) }
}

/// FUID is semantically identical to TUID for our purposes (IID/CID).
pub type Fuid = Tuid;

/// Helper macro to define a `Tuid` from 16 literal bytes in COM layout.
#[macro_export]
macro_rules! tuid {
    ($b0:expr,$b1:expr,$b2:expr,$b3:expr,$b4:expr,$b5:expr,$b6:expr,$b7:expr,$b8:expr,$b9:expr,$bA:expr,$bB:expr,$bC:expr,$bD:expr,$bE:expr,$bF:expr) => {
        $crate::Tuid::new([$b0,$b1,$b2,$b3,$b4,$b5,$b6,$b7,$b8,$b9,$bA,$bB,$bC,$bD,$bE,$bF])
    };
}

// ----- FUnknown (base interface; COM-like) -----------------------------------
//
// Methods are function pointers in a vtable with C calling convention.
//   query_interface(self, iid, obj_out) -> tresult
//   add_ref(self) -> u32
//   release(self) -> u32
//
// NOTE: We purposely avoid any C++ ABI assumptions: this is a C layout vtable.

#[repr(C)]
pub struct FUnknownVTable {
    pub query_interface: unsafe extern "C" fn(this_: *mut FUnknown, iid: *const Fuid, obj: *mut *mut c_void) -> tresult,
    pub add_ref: unsafe extern "C" fn(this_: *mut FUnknown) -> u32,
    pub release: unsafe extern "C" fn(this_: *mut FUnknown) -> u32,
}

#[repr(C)]
pub struct FUnknown {
    pub vtbl: *const FUnknownVTable,
}

impl FUnknown {
    #[inline]
    pub unsafe fn query_interface<T>(&mut self, iid: &Fuid, out: *mut *mut T) -> tresult {
        ((*self.vtbl).query_interface)(self, iid as *const _ as *const Fuid, out as *mut *mut c_void)
    }
    #[inline]
    pub unsafe fn add_ref(&mut self) -> u32 {
        ((*self.vtbl).add_ref)(self)
    }
    #[inline]
    pub unsafe fn release(&mut self) -> u32 {
        ((*self.vtbl).release)(self)
    }
}

// ----- IPluginFactory (Phase 1) ----------------------------------------------
//
// Public docs describe the factory as the anchor provided by GetPluginFactory().
// Methods we rely on here:
//   countClasses() -> int32
//   createInstance(class_id: *const TUID, iid: *const TUID, obj: **void) -> tresult
//
// We deliberately postpone getClassInfo*/PClassInfo layouts to Phase 2.

#[repr(C)]
pub struct IPluginFactoryVTable {
    // FUnknown base (must be first / same layout)
    pub query_interface: unsafe extern "C" fn(this_: *mut FUnknown, iid: *const Fuid, obj: *mut *mut c_void) -> tresult,
    pub add_ref: unsafe extern "C" fn(this_: *mut FUnknown) -> u32,
    pub release: unsafe extern "C" fn(this_: *mut FUnknown) -> u32,

    // IPluginFactory
    pub count_classes: unsafe extern "C" fn(this_: *mut IPluginFactory) -> int32,

    // We will bind getClassInfo* in Phase 2
    pub _get_class_info: *const c_void,
    pub _get_class_info2: *const c_void,
    pub _get_class_info3: *const c_void,

    pub create_instance: unsafe extern "C" fn(
        this_: *mut IPluginFactory,
        cid: *const Tuid,
        iid: *const Tuid,
        obj: *mut *mut c_void,
    ) -> tresult,
}

#[repr(C)]
pub struct IPluginFactory {
    pub vtbl: *const IPluginFactoryVTable,
}

impl IPluginFactory {
    #[inline]
    pub unsafe fn as_funknown(&mut self) -> &mut FUnknown {
        &mut *(self as *mut _ as *mut FUnknown)
    }
    #[inline]
    pub unsafe fn count_classes(&mut self) -> int32 {
        ((*self.vtbl).count_classes)(self)
    }
    #[inline]
    pub unsafe fn create_instance_raw(
        &mut self,
        cid: &Tuid,
        iid: &Tuid,
        obj: *mut *mut c_void,
    ) -> tresult {
        ((*self.vtbl).create_instance)(self, cid as *const _, iid as *const _, obj)
    }
}

/// Type of the module entry point symbol: `GetPluginFactory`
///
/// Public docs: a C-style export named `GetPluginFactory` returning the factory.
/// We intentionally use `extern "C"` here (not "system") to match cross-platform
/// expectations in public examples.
pub type GetPluginFactoryProc = unsafe extern "C" fn() -> *mut IPluginFactory;

// Safe wrapper for non-null factory pointer
#[derive(Copy, Clone)]
pub struct FactoryHandle(NonNull<IPluginFactory>);

impl FactoryHandle {
    pub unsafe fn new(ptr: *mut IPluginFactory) -> Option<Self> {
        NonNull::new(ptr).map(Self)
    }
    pub fn as_mut(&self) -> &mut IPluginFactory {
        unsafe { self.0.as_ptr().as_mut().unwrap() }
    }
}

// Marker types for IIDs/CIDs we’ll introduce in later phases.
pub struct InterfaceId(pub Fuid);
pub struct ClassId(pub Fuid);

// Prevent Send/Sync by default, these are raw interface pointers.
unsafe impl Send for FactoryHandle {}
unsafe impl Sync for FactoryHandle {}

