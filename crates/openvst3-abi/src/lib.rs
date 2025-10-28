#![cfg_attr(not(feature = "std"), no_std)]
#![allow(non_camel_case_types)]
//! Clean-room, header-free ABI surfaces for VST3 hosting.
//! Phase 1: FUnknown, IPluginFactory
//! Phase 2: PClassInfo
//! Phase 3: IPluginBase, IComponent, IAudioProcessor + processing structs (minimal)

use core::ffi::c_void;
use core::ptr::NonNull;

// ----- Core scalar types ------------------------------------------------------
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

/// 16-byte type used for IIDs/CIDs.
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
    ($($b:expr),* $(,)?) => { $crate::Tuid::new([ $($b as u8),* ]) };
}

// ===== FUnknown ===============================================================
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
        ((*self.vtbl).query_interface)(self, iid as *const _, out as *mut *mut c_void)
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

// ===== Class info (Phase 2) ===================================================
pub mod classinfo_consts {
    pub const K_NAME_SIZE: usize = 64;
    pub const K_CATEGORY_SIZE: usize = 32;
    pub const K_VENDOR_SIZE: usize = 64;
    pub const K_VERSION_SIZE: usize = 64;
    pub const K_SUBCATS_SIZE: usize = 128;
}

#[repr(C)]
pub struct PClassInfo {
    pub cid: [i8; 16],
    pub cardinality: int32,
    pub category: [i8; classinfo_consts::K_CATEGORY_SIZE],
    pub name: [i8; classinfo_consts::K_NAME_SIZE],
}

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

// ===== IPluginFactory =========================================================
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

    // v1
    pub get_factory_info:
        unsafe extern "C" fn(this_: *mut IPluginFactory, info: *mut c_void) -> tresult, // reserved
    pub count_classes: unsafe extern "C" fn(this_: *mut IPluginFactory) -> int32,
    pub get_class_info: unsafe extern "C" fn(
        this_: *mut IPluginFactory,
        index: int32,
        info: *mut PClassInfo,
    ) -> tresult,

    // create
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

pub type GetPluginFactoryProc = unsafe extern "C" fn() -> *mut IPluginFactory;

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
unsafe impl Send for FactoryHandle {}
unsafe impl Sync for FactoryHandle {}

// ===== Phase 3: Processing surfaces ==========================================
//
// NOTE: These are *minimal* signatures/types to enable a null-host test.
// We avoid copying header text; names and roles follow public docs.
//
// --- enums/consts (symbolic sample type, process mode, flags) ---
pub mod process_consts {
    pub const SYMBOLIC_SAMPLE_32: i32 = 0;
    pub const SYMBOLIC_SAMPLE_64: i32 = 1;

    pub const PROCESS_MODE_REALTIME: i32 = 0;
    pub const PROCESS_MODE_PREFETCH: i32 = 1;

    pub const PROCESS_SETUP_HAS_TAIL: i32 = 1 << 0;
}

pub type Sample32 = f32;
pub type Sample64 = f64;

// --- ProcessSetup ---
#[repr(C)]
pub struct ProcessSetup {
    pub process_mode: int32, // symbolic
    pub sample_rate: f64,
    pub max_samples_per_block: int32,
    pub symbolic_sample_size: int32, // 0=32f, 1=64f
    pub flags: int32,                // optional features
}

// --- AudioBusBuffers (32-bit path only for now) ---
#[repr(C)]
pub struct AudioBusBuffers32 {
    pub num_channels: int32,
    pub silence_flags: uint64,               // bit per channel
    pub channel_buffers: *mut *mut Sample32, // [num_channels][num_samples]
}

// --- ProcessData (trimmed: audio only, 32-bit) ---
#[repr(C)]
pub struct ProcessData32 {
    pub num_inputs: int32,
    pub num_outputs: int32,
    pub inputs: *mut AudioBusBuffers32,
    pub outputs: *mut AudioBusBuffers32,
    pub num_samples: int32,
    // Skipping events/parameters for Phase 3 boot
}

// --- IPluginBase ---
#[repr(C)]
pub struct IPluginBaseVTable {
    pub query_interface: unsafe extern "C" fn(
        this_: *mut FUnknown,
        iid: *const Fuid,
        obj: *mut *mut c_void,
    ) -> tresult,
    pub add_ref: unsafe extern "C" fn(this_: *mut FUnknown) -> u32,
    pub release: unsafe extern "C" fn(this_: *mut FUnknown) -> u32,

    pub initialize:
        unsafe extern "C" fn(this_: *mut IPluginBase, context: *mut FUnknown) -> tresult,
    pub terminate: unsafe extern "C" fn(this_: *mut IPluginBase) -> tresult,
}

#[repr(C)]
pub struct IPluginBase {
    pub vtbl: *const IPluginBaseVTable,
}
impl IPluginBase {
    #[inline]
    pub unsafe fn initialize(&mut self, ctx: *mut FUnknown) -> tresult {
        ((*self.vtbl).initialize)(self, ctx)
    }
    #[inline]
    pub unsafe fn terminate(&mut self) -> tresult {
        ((*self.vtbl).terminate)(self)
    }
}

// --- IComponent (subset used by host boot) ---
#[repr(C)]
pub struct IComponentVTable {
    pub query_interface: unsafe extern "C" fn(
        this_: *mut FUnknown,
        iid: *const Fuid,
        obj: *mut *mut c_void,
    ) -> tresult,
    pub add_ref: unsafe extern "C" fn(this_: *mut FUnknown) -> u32,
    pub release: unsafe extern "C" fn(this_: *mut FUnknown) -> u32,

    // IPluginBase
    pub initialize: unsafe extern "C" fn(this_: *mut IComponent, context: *mut FUnknown) -> tresult,
    pub terminate: unsafe extern "C" fn(this_: *mut IComponent) -> tresult,

    // Minimal subset of IComponent methods we’ll likely use later:
    pub get_controller_class_id:
        unsafe extern "C" fn(this_: *mut IComponent, cid: *mut Tuid) -> tresult,
    // (more methods come later)
}

#[repr(C)]
pub struct IComponent {
    pub vtbl: *const IComponentVTable,
}
impl IComponent {
    #[inline]
    pub unsafe fn initialize(&mut self, ctx: *mut FUnknown) -> tresult {
        ((*self.vtbl).initialize)(self, ctx)
    }
    #[inline]
    pub unsafe fn terminate(&mut self) -> tresult {
        ((*self.vtbl).terminate)(self)
    }
    #[inline]
    pub unsafe fn get_controller_class_id(&mut self, cid: *mut Tuid) -> tresult {
        ((*self.vtbl).get_controller_class_id)(self, cid)
    }
}

// --- IAudioProcessor (subset to run a null block, 32-bit float only) ---
#[repr(C)]
pub struct IAudioProcessorVTable {
    pub query_interface: unsafe extern "C" fn(
        this_: *mut FUnknown,
        iid: *const Fuid,
        obj: *mut *mut c_void,
    ) -> tresult,
    pub add_ref: unsafe extern "C" fn(this_: *mut FUnknown) -> u32,
    pub release: unsafe extern "C" fn(this_: *mut FUnknown) -> u32,

    // IPluginBase
    pub initialize:
        unsafe extern "C" fn(this_: *mut IAudioProcessor, context: *mut FUnknown) -> tresult,
    pub terminate: unsafe extern "C" fn(this_: *mut IAudioProcessor) -> tresult,

    // Minimal processing path
    pub set_processing: unsafe extern "C" fn(this_: *mut IAudioProcessor, state: i32) -> tresult, // 0/1
    pub setup_processing:
        unsafe extern "C" fn(this_: *mut IAudioProcessor, setup: *const ProcessSetup) -> tresult,
    pub process_32f:
        unsafe extern "C" fn(this_: *mut IAudioProcessor, data: *mut ProcessData32) -> tresult,
    // (bus arrangement etc. can come later)
}

#[repr(C)]
pub struct IAudioProcessor {
    pub vtbl: *const IAudioProcessorVTable,
}
impl IAudioProcessor {
    #[inline]
    pub unsafe fn initialize(&mut self, ctx: *mut FUnknown) -> tresult {
        ((*self.vtbl).initialize)(self, ctx)
    }
    #[inline]
    pub unsafe fn terminate(&mut self) -> tresult {
        ((*self.vtbl).terminate)(self)
    }
    #[inline]
    pub unsafe fn set_processing(&mut self, state: i32) -> tresult {
        ((*self.vtbl).set_processing)(self, state)
    }
    #[inline]
    pub unsafe fn setup_processing(&mut self, s: &ProcessSetup) -> tresult {
        ((*self.vtbl).setup_processing)(self, s as *const _)
    }
    #[inline]
    pub unsafe fn process_32f(&mut self, d: &mut ProcessData32) -> tresult {
        ((*self.vtbl).process_32f)(self, d as *mut _)
    }
}
