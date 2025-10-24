// FFI to the C shim + dynamic loader for GetPluginFactory
#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]

use libloading::{Library, Symbol};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct v3_class_info {
    pub category: [u8; 64],
    pub name: [u8; 128],
    pub cid: [u8; 16],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct v3_bus_info {
    pub media_type: i32,
    pub direction: i32,
    pub channel_count: i32,
    pub bus_type: i32,
    pub flags: u32,
    pub name: [u8; 128],
}

pub type v3_factory = *mut core::ffi::c_void;
pub type v3_component = *mut core::ffi::c_void;
pub type v3_audio_processor = *mut core::ffi::c_void;
pub type v3_funknown = *mut core::ffi::c_void;
pub type v3_speaker_arrangement = u64;

pub const MEDIA_TYPE_AUDIO: i32 = 0;
pub const MEDIA_TYPE_EVENT: i32 = 1;

pub const BUS_DIRECTION_INPUT: i32 = 0;
pub const BUS_DIRECTION_OUTPUT: i32 = 1;

pub const BUS_TYPE_MAIN: i32 = 0;
pub const BUS_TYPE_AUX: i32 = 1;

pub const BUS_FLAG_DEFAULT_ACTIVE: u32 = 1 << 0;
pub const BUS_FLAG_IS_CONTROL_VOLTAGE: u32 = 1 << 1;

extern "C" {
    pub fn v3_factory_class_count(f: v3_factory) -> i32;
    pub fn v3_factory_class_info(f: v3_factory, idx: i32, out_info: *mut v3_class_info) -> i32;
    pub fn v3_factory_create_audio_processor(
        f: v3_factory,
        cid: *const u8,
        out_proc: *mut v3_audio_processor,
        out_comp: *mut v3_component,
    ) -> i32;

    pub fn v3_release(obj: v3_funknown) -> i32;

    pub fn v3_component_initialize(c: v3_component) -> i32;
    pub fn v3_component_set_active(c: v3_component, state: i32) -> i32;
    pub fn v3_component_terminate(c: v3_component) -> i32;
    pub fn v3_component_get_bus_count(c: v3_component, media_type: i32, direction: i32) -> i32;
    pub fn v3_component_get_bus_info(
        c: v3_component,
        media_type: i32,
        direction: i32,
        index: i32,
        out_info: *mut v3_bus_info,
    ) -> i32;
    pub fn v3_component_activate_bus(
        c: v3_component,
        media_type: i32,
        direction: i32,
        index: i32,
        state: i32,
    ) -> i32;

    pub fn v3_audio_processor_setup(
        p: v3_audio_processor,
        sample_rate: f64,
        max_block: i32,
        in_ch: i32,
        out_ch: i32,
    ) -> i32;
    pub fn v3_audio_processor_set_active(p: v3_audio_processor, state: i32) -> i32;
    pub fn v3_audio_processor_set_processing(p: v3_audio_processor, state: i32) -> i32;
    pub fn v3_audio_processor_get_bus_arrangements(
        p: v3_audio_processor,
        in_count: i32,
        inputs: *mut v3_speaker_arrangement,
        out_count: i32,
        outputs: *mut v3_speaker_arrangement,
    ) -> i32;
    pub fn v3_audio_processor_set_bus_arrangements(
        p: v3_audio_processor,
        in_count: i32,
        inputs: *const v3_speaker_arrangement,
        out_count: i32,
        outputs: *const v3_speaker_arrangement,
    ) -> i32;

    pub fn v3_audio_processor_process_f32(
        p: v3_audio_processor,
        inputs: *const *const f32,
        in_ch: i32,
        outputs: *mut *mut f32,
        out_ch: i32,
        num_samples: i32,
    ) -> i32;
}

// Loader for GetPluginFactory
pub type GetPluginFactoryFn = unsafe extern "C" fn() -> v3_factory;

pub struct Vst3Lib {
    pub lib: Library,
    pub get_factory: Symbol<GetPluginFactoryFn>,
}
impl Vst3Lib {
    pub unsafe fn load<P: AsRef<std::ffi::OsStr>>(path: P) -> Result<Self, libloading::Error> {
        let lib = Library::new(path)?;
        let get_factory: Symbol<GetPluginFactoryFn> = lib.get(b"GetPluginFactory\0")?;
        Ok(Self { lib, get_factory })
    }
}
