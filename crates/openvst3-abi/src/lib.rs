#![cfg_attr(not(feature = "std"), no_std)]
#![allow(non_camel_case_types)]
//! Clean-room, header-free minimal ABI for VST3 hosting & modules.
//! Phase 1: FUnknown, IPluginFactory, GetPluginFactory
//! Phase 2: PClassInfo/PClassInfo2 + IPluginFactory::getClassInfo

use core::ffi::c_void;
use core::ptr::NonNull;

// ----- Core scalar types -----
pub type int16 = i16;
pub type int32 = i32;
pub type int64 = i64;
pub type uint32 = u32;
pub type uint64 = u64;
pub type tresult = int32;

pub const K_RESULT_OK: tresult = 0;
pub const K_RESULT_FALSE: tresult = 1;
pub const K_NOT_IMPLEMENTED: tresult = -1;
pub const K_NO_INTERFACE: tresult = -2;
pub const K_INVALID_ARG: tresult = -3;
pub const K_INTERNAL_ERR: tresult = -4;

/// 16-byte type used for IIDs/CIDs (TUID/FUID).
#[repr(C)]
#[derive(Copy, Clone, Eq, PartialEq)]
pub struct Tuid(pub [u8; 16]);
pub type Fuid = Tuid;

impl Tuid {
    pub const fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
}

#[macro_export]
macro_rules! tuid {
    ($b0:expr,$b1:expr,$b2:expr,$b3:expr,$b4:expr,$b5:expr,$b6:expr,$b7:expr,$b8:expr,$b9:expr,$bA:expr,$bB:expr,$bC:expr,$bD:expr,$bE:expr,$bF:expr) => {
        $crate::Tuid::new([
            $b0, $b1, $b2, $b3, $b4, $b5, $b6, $b7, $b8, $b9, $bA, $bB, $bC, $bD, $bE, $bF,
        ])
    };
}

// ===== FUnknown =====
#[repr(C)]
pub struct FUnknownVTable {
    pub query_interface: unsafe extern "C" fn(
        this_: *mut FUnknown,
        iid: *const Fuid,
        obj: *mut *mut c_void,
    ) -> tresult,
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
        ((*self.vtbl).query_interface)(
            self,
            iid as *const _ as *const Fuid,
            out as *mut *mut c_void,
        )
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

// ===== Class info structs =====
//
// The public docs show PClassInfo2 fields and sizes (vendor/version/subcats) and
// refer to PClassInfo::kNameSize and kCategorySize. In practice, hosts should
// accept at least 64 for name and 32 for category (commonly used values).
// We define numeric constants accordingly for our ABI boundary.
pub mod classinfo_consts {
    pub const K_NAME_SIZE: usize = 64; // PClassInfo::kNameSize (commonly 64)
    pub const K_CATEGORY_SIZE: usize = 32; // PClassInfo::kCategorySize (commonly 32)
    pub const K_VENDOR_SIZE: usize = 64; // PClassInfo2::kVendorSize
    pub const K_VERSION_SIZE: usize = 64; // PClassInfo2::kVersionSize
    pub const K_SUBCATS_SIZE: usize = 128; // PClassInfo2::kSubCategoriesSize
}

/// PClassInfo (version 1) per public docs: CID, cardinality, category, name.
#[repr(C)]
pub struct PClassInfo {
    pub cid: [i8; 16], // raw 16-byte CID
    pub cardinality: int32,
    pub category: [i8; classinfo_consts::K_CATEGORY_SIZE],
    pub name: [i8; classinfo_consts::K_NAME_SIZE],
}

/// PClassInfo2 extends PClassInfo with flags/vendor/version/sdkVersion/subCategories.
#[repr(C)]
pub struct PClassInfo2 {
    pub cid: [i8; 16],
    pub cardinality: int32,
    pub category: [i8; classinfo_consts::K_CATEGORY_SIZE],
    pub name: [i8; classinfo_consts::K_NAME_SIZE],
    pub class_flags: u32,
    pub sub_categories: [i8; classinfo_consts::K_SUBCATS_SIZE],
    pub vendor: [i8; classinfo_consts::K_VENDOR_SIZE],
    pub version: [i8; classinfo_consts::K_VERSION_SIZE],
    pub sdk_version: [i8; classinfo_consts::K_VERSION_SIZE],
}

// ===== IPluginFactory =====
#[repr(C)]
pub struct IPluginFactoryVTable {
    // FUnknown base
    pub query_interface: unsafe extern "C" fn(
        this_: *mut FUnknown,
        iid: *const Fuid,
        obj: *mut *mut c_void,
    ) -> tresult,
    pub add_ref: unsafe extern "C" fn(this_: *mut FUnknown) -> u32,
    pub release: unsafe extern "C" fn(this_: *mut FUnknown) -> u32,

    // IPluginFactory v1
    pub get_factory_info:
        unsafe extern "C" fn(this_: *mut IPluginFactory, info: *mut c_void) -> tresult, // we’ll bind PFactoryInfo later
    pub count_classes: unsafe extern "C" fn(this_: *mut IPluginFactory) -> int32,
    pub get_class_info: unsafe extern "C" fn(
        this_: *mut IPluginFactory,
        index: int32,
        info: *mut PClassInfo,
    ) -> tresult,

    // createInstance
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
    pub unsafe fn get_class_info(&mut self, index: int32, out: *mut PClassInfo) -> tresult {
        ((*self.vtbl).get_class_info)(self, index, out)
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

/// Signature of the module entry point: `GetPluginFactory`
pub type GetPluginFactoryProc = unsafe extern "C" fn() -> *mut IPluginFactory;

// Non-null wrapper
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

// Marker wrappers (future use)
pub struct InterfaceId(pub Fuid);
pub struct ClassId(pub Fuid);

// Allow moving across threads (host responsibility to call on correct threads)
unsafe impl Send for FactoryHandle {}
unsafe impl Sync for FactoryHandle {}
