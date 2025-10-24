use anyhow::{anyhow, Context, Result};
use bitflags::bitflags;
use openvst3_sys as sys;
use std::mem::MaybeUninit;
use std::ptr;

pub struct LoadedFactory {
    _lib: sys::Vst3Lib,
    pub factory: sys::v3_factory,
}
impl LoadedFactory {
    /// Load a .so (binary inside .vst3) and obtain the factory pointer.
    pub unsafe fn load_plugin<P: AsRef<std::ffi::OsStr>>(path: P) -> Result<Self> {
        let lib = sys::Vst3Lib::load(path)?;
        let f = (lib.get_factory)();
        if f.is_null() {
            return Err(anyhow!("GetPluginFactory returned null"));
        }
        Ok(Self {
            _lib: lib,
            factory: f,
        })
    }
    pub fn classes(&self) -> Result<Vec<ClassInfo>> {
        let n = unsafe { sys::v3_factory_class_count(self.factory) };
        if n < 0 {
            return Err(anyhow!("class_count rc={n}"));
        }
        let mut out = Vec::new();
        for i in 0..n {
            let mut c = sys::v3_class_info {
                category: [0; 64],
                name: [0; 128],
                cid: [0; 16],
            };
            let rc = unsafe { sys::v3_factory_class_info(self.factory, i, &mut c as *mut _) };
            if rc == 0 {
                let cat = c.category.split(|&b| b == 0).next().unwrap_or(&[]);
                let name = c.name.split(|&b| b == 0).next().unwrap_or(&[]);
                out.push(ClassInfo {
                    category: String::from_utf8_lossy(cat).to_string(),
                    name: String::from_utf8_lossy(name).to_string(),
                    cid: c.cid,
                });
            }
        }
        Ok(out)
    }
    pub fn create_audio_processor(&self, cid: [u8; 16]) -> Result<AudioProcessor> {
        let mut proc: sys::v3_audio_processor = ptr::null_mut();
        let mut comp: sys::v3_component = ptr::null_mut();
        let rc = unsafe {
            sys::v3_factory_create_audio_processor(
                self.factory,
                cid.as_ptr(),
                &mut proc as *mut _,
                &mut comp as *mut _,
            )
        };
        if rc != 0 || proc.is_null() || comp.is_null() {
            return Err(anyhow!("create_audio_processor rc={rc}"));
        }
        Ok(AudioProcessor {
            comp,
            proc,
            active: false,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ClassInfo {
    pub category: String,
    pub name: String,
    pub cid: [u8; 16],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaType {
    Audio,
    Event,
}
impl MediaType {
    fn to_raw(self) -> i32 {
        match self {
            MediaType::Audio => sys::MEDIA_TYPE_AUDIO,
            MediaType::Event => sys::MEDIA_TYPE_EVENT,
        }
    }
    fn from_raw(raw: i32) -> Option<Self> {
        match raw {
            x if x == sys::MEDIA_TYPE_AUDIO => Some(MediaType::Audio),
            x if x == sys::MEDIA_TYPE_EVENT => Some(MediaType::Event),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BusDirection {
    Input,
    Output,
}
impl BusDirection {
    fn to_raw(self) -> i32 {
        match self {
            BusDirection::Input => sys::BUS_DIRECTION_INPUT,
            BusDirection::Output => sys::BUS_DIRECTION_OUTPUT,
        }
    }
    fn from_raw(raw: i32) -> Option<Self> {
        match raw {
            x if x == sys::BUS_DIRECTION_INPUT => Some(BusDirection::Input),
            x if x == sys::BUS_DIRECTION_OUTPUT => Some(BusDirection::Output),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BusType {
    Main,
    Aux,
    Other(i32),
}
impl BusType {
    fn from_raw(raw: i32) -> Self {
        match raw {
            x if x == sys::BUS_TYPE_MAIN => BusType::Main,
            x if x == sys::BUS_TYPE_AUX => BusType::Aux,
            other => BusType::Other(other),
        }
    }
}

bitflags! {
    pub struct BusFlags: u32 {
        const DEFAULT_ACTIVE = sys::BUS_FLAG_DEFAULT_ACTIVE;
        const IS_CONTROL_VOLTAGE = sys::BUS_FLAG_IS_CONTROL_VOLTAGE;
    }
}

#[derive(Debug, Clone)]
pub struct BusInfo {
    pub media_type: MediaType,
    pub direction: BusDirection,
    pub channel_count: i32,
    pub bus_type: BusType,
    pub flags: BusFlags,
    pub name: String,
}
impl BusInfo {
    fn from_raw(raw: sys::v3_bus_info) -> Self {
        let name_end = raw
            .name
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(raw.name.len());
        let name = String::from_utf8_lossy(&raw.name[..name_end]).to_string();
        Self {
            media_type: MediaType::from_raw(raw.media_type).unwrap_or(MediaType::Audio),
            direction: BusDirection::from_raw(raw.direction).unwrap_or(BusDirection::Input),
            channel_count: raw.channel_count,
            bus_type: BusType::from_raw(raw.bus_type),
            flags: BusFlags::from_bits_retain(raw.flags),
            name,
        }
    }
}

pub struct AudioProcessor {
    comp: sys::v3_component,
    proc: sys::v3_audio_processor,
    active: bool,
}
unsafe impl Send for AudioProcessor {}
unsafe impl Sync for AudioProcessor {}

impl AudioProcessor {
    pub fn initialize(&mut self) -> Result<()> {
        let rc = unsafe { sys::v3_component_initialize(self.comp) };
        if rc != 0 {
            return Err(anyhow!("component_initialize rc={rc}"));
        }
        Ok(())
    }
    pub fn setup(&mut self, sr: f64, max_block: i32, in_ch: i32, out_ch: i32) -> Result<()> {
        let rc = unsafe { sys::v3_audio_processor_setup(self.proc, sr, max_block, in_ch, out_ch) };
        if rc != 0 {
            return Err(anyhow!("setupProcessing rc={rc}"));
        }
        self.reapply_default_arrangements()?;
        self.activate_default_audio_buses(in_ch, out_ch)?;
        Ok(())
    }
    pub fn set_active(&mut self, on: bool) -> Result<()> {
        if on {
            let rc_comp = unsafe { sys::v3_component_set_active(self.comp, 1) };
            let rc_proc = unsafe { sys::v3_audio_processor_set_active(self.proc, 1) };
            let rc_processing = unsafe { sys::v3_audio_processor_set_processing(self.proc, 1) };
            if rc_comp != 0 || rc_proc != 0 || rc_processing != 0 {
                return Err(anyhow!(
                    "setActive rc comp={rc_comp} proc={rc_proc} processing={rc_processing}"
                ));
            }
        } else {
            let rc_processing = unsafe { sys::v3_audio_processor_set_processing(self.proc, 0) };
            let rc_proc = unsafe { sys::v3_audio_processor_set_active(self.proc, 0) };
            let rc_comp = unsafe { sys::v3_component_set_active(self.comp, 0) };
            if rc_comp != 0 || rc_proc != 0 || rc_processing != 0 {
                return Err(anyhow!(
                    "setActive rc comp={rc_comp} proc={rc_proc} processing={rc_processing}"
                ));
            }
        }
        self.active = on;
        Ok(())
    }
    /// Process in-place with deinterleaved channel pointers.
    pub fn process_f32(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        nframes: usize,
    ) -> Result<()> {
        if !self.active {
            return Err(anyhow!("processor not active"));
        }
        if inputs.iter().any(|ch| ch.len() < nframes) {
            return Err(anyhow!("input buffer smaller than requested frames"));
        }
        if outputs.iter().any(|ch| ch.len() < nframes) {
            return Err(anyhow!("output buffer smaller than requested frames"));
        }
        // Build pointer arrays
        let in_ptrs: Vec<*const f32> = inputs.iter().map(|ch| ch.as_ptr()).collect();
        let mut out_ptrs: Vec<*mut f32> = outputs.iter_mut().map(|ch| ch.as_mut_ptr()).collect();
        let rc = unsafe {
            sys::v3_audio_processor_process_f32(
                self.proc,
                if in_ptrs.is_empty() {
                    ptr::null()
                } else {
                    in_ptrs.as_ptr()
                },
                inputs.len() as i32,
                if out_ptrs.is_empty() {
                    ptr::null_mut()
                } else {
                    out_ptrs.as_mut_ptr()
                },
                outputs.len() as i32,
                nframes as i32,
            )
        };
        if rc != 0 {
            return Err(anyhow!("process rc={rc}"));
        }
        Ok(())
    }

    pub fn bus_count(&self, media: MediaType, direction: BusDirection) -> Result<i32> {
        let count = unsafe {
            sys::v3_component_get_bus_count(self.comp, media.to_raw(), direction.to_raw())
        };
        if count < 0 {
            return Err(anyhow!("get_bus_count rc={count}"));
        }
        Ok(count)
    }

    pub fn bus_info(
        &self,
        media: MediaType,
        direction: BusDirection,
        index: i32,
    ) -> Result<BusInfo> {
        let mut raw = MaybeUninit::<sys::v3_bus_info>::uninit();
        let rc = unsafe {
            sys::v3_component_get_bus_info(
                self.comp,
                media.to_raw(),
                direction.to_raw(),
                index,
                raw.as_mut_ptr(),
            )
        };
        if rc != 0 {
            return Err(anyhow!("get_bus_info rc={rc}"));
        }
        let raw = unsafe { raw.assume_init() };
        Ok(BusInfo::from_raw(raw))
    }

    pub fn activate_bus(
        &mut self,
        media: MediaType,
        direction: BusDirection,
        index: i32,
        state: bool,
    ) -> Result<()> {
        let rc = unsafe {
            sys::v3_component_activate_bus(
                self.comp,
                media.to_raw(),
                direction.to_raw(),
                index,
                if state { 1 } else { 0 },
            )
        };
        if rc != 0 {
            return Err(anyhow!("activate_bus rc={rc}"));
        }
        Ok(())
    }

    pub fn get_bus_arrangements(
        &self,
    ) -> Result<(
        Vec<sys::v3_speaker_arrangement>,
        Vec<sys::v3_speaker_arrangement>,
    )> {
        let in_count = self
            .bus_count(MediaType::Audio, BusDirection::Input)?
            .max(0) as usize;
        let out_count = self
            .bus_count(MediaType::Audio, BusDirection::Output)?
            .max(0) as usize;
        let mut inputs = vec![0; in_count];
        let mut outputs = vec![0; out_count];
        let rc = unsafe {
            sys::v3_audio_processor_get_bus_arrangements(
                self.proc,
                inputs.len() as i32,
                if inputs.is_empty() {
                    ptr::null_mut()
                } else {
                    inputs.as_mut_ptr()
                },
                outputs.len() as i32,
                if outputs.is_empty() {
                    ptr::null_mut()
                } else {
                    outputs.as_mut_ptr()
                },
            )
        };
        if rc != 0 {
            return Err(anyhow!("get_bus_arrangements rc={rc}"));
        }
        Ok((inputs, outputs))
    }

    pub fn set_bus_arrangements(
        &mut self,
        inputs: &[sys::v3_speaker_arrangement],
        outputs: &[sys::v3_speaker_arrangement],
    ) -> Result<()> {
        let rc = unsafe {
            sys::v3_audio_processor_set_bus_arrangements(
                self.proc,
                inputs.len() as i32,
                if inputs.is_empty() {
                    ptr::null()
                } else {
                    inputs.as_ptr()
                },
                outputs.len() as i32,
                if outputs.is_empty() {
                    ptr::null()
                } else {
                    outputs.as_ptr()
                },
            )
        };
        if rc != 0 {
            return Err(anyhow!("set_bus_arrangements rc={rc}"));
        }
        Ok(())
    }

    fn reapply_default_arrangements(&mut self) -> Result<()> {
        let (ins, outs) = self.get_bus_arrangements()?;
        if !ins.is_empty() || !outs.is_empty() {
            self.set_bus_arrangements(&ins, &outs)?;
        }
        Ok(())
    }

    fn activate_default_audio_buses(&mut self, in_ch: i32, out_ch: i32) -> Result<()> {
        let configs = [
            (MediaType::Audio, BusDirection::Input, in_ch),
            (MediaType::Audio, BusDirection::Output, out_ch),
        ];
        for (media, direction, host_channels) in configs {
            let count = self.bus_count(media, direction)?;
            for idx in 0..count {
                let info = self.bus_info(media, direction, idx)?;
                let should_activate = host_channels > 0 && info.channel_count > 0;
                self.activate_bus(media, direction, idx, should_activate)?;
            }
        }
        Ok(())
    }
}
impl Drop for AudioProcessor {
    fn drop(&mut self) {
        unsafe {
            let _ = sys::v3_audio_processor_set_processing(self.proc, 0);
            let _ = sys::v3_audio_processor_set_active(self.proc, 0);
            let _ = sys::v3_component_set_active(self.comp, 0);
            let _ = sys::v3_component_terminate(self.comp);
            let _ = sys::v3_release(self.proc as _);
            let _ = sys::v3_release(self.comp as _);
        }
    }
}
