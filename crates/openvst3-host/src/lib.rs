use anyhow::{anyhow, Context, Result};
use openvst3_sys as sys;

pub struct LoadedFactory {
    _lib: sys::Vst3Lib,
    pub factory: sys::v3_factory,
}
impl LoadedFactory {
    /// Load a .so (binary inside .vst3) and obtain the factory pointer.
    pub unsafe fn load_plugin<P: AsRef<std::ffi::OsStr>>(path: P) -> Result<Self> {
        let lib = sys::Vst3Lib::load(path)?;
        let f = (lib.get_factory)();
        if f.is_null() { return Err(anyhow!("GetPluginFactory returned null")); }
        Ok(Self { _lib: lib, factory: f })
    }
    pub fn classes(&self) -> Result<Vec<ClassInfo>> {
        let n = unsafe { sys::v3_factory_class_count(self.factory) };
        if n < 0 { return Err(anyhow!("class_count rc={n}")); }
        let mut out = Vec::new();
        for i in 0..n {
            let mut c = sys::v3_class_info { category: [0;64], name: [0;128], cid: [0;16] };
            let rc = unsafe { sys::v3_factory_class_info(self.factory, i, &mut c as *mut _) };
            if rc == 0 {
                let cat = c.category.split(|&b| b==0).next().unwrap_or(&[]);
                let name = c.name.split(|&b| b==0).next().unwrap_or(&[]);
                out.push(ClassInfo {
                    category: String::from_utf8_lossy(cat).to_string(),
                    name: String::from_utf8_lossy(name).to_string(),
                    cid: c.cid,
                });
            }
        }
        Ok(out)
    }
    pub fn create_audio_processor(&self, cid: [u8;16]) -> Result<AudioProcessor> {
        let mut proc: sys::v3_audio_processor = std::ptr::null_mut();
        let mut comp: sys::v3_component = std::ptr::null_mut();
        let rc = unsafe { sys::v3_factory_create_audio_processor(self.factory, cid.as_ptr(), &mut proc as *mut _, &mut comp as *mut _) };
        if rc != 0 || proc.is_null() || comp.is_null() { return Err(anyhow!("create_audio_processor rc={rc}")); }
        Ok(AudioProcessor { comp, proc, active: false })
    }
}

pub struct ClassInfo {
    pub category: String,
    pub name: String,
    pub cid: [u8; 16],
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
        if rc != 0 { return Err(anyhow!("component_initialize rc={rc}")); }
        Ok(())
    }
    pub fn setup(&mut self, sr: f64, max_block: i32, in_ch: i32, out_ch: i32) -> Result<()> {
        let rc = unsafe { sys::v3_audio_processor_setup(self.proc, sr, max_block, in_ch, out_ch) };
        if rc != 0 { return Err(anyhow!("setupProcessing rc={rc}")); }
        Ok(())
    }
    pub fn set_active(&mut self, on: bool) -> Result<()> {
        let rc1 = unsafe { sys::v3_audio_processor_set_active(self.proc, if on {1} else {0}) };
        let rc2 = unsafe { sys::v3_component_set_active(self.comp, if on {1} else {0}) };
        if rc1 != 0 || rc2 != 0 { return Err(anyhow!("setActive rc proc={rc1} comp={rc2}")); }
        self.active = on;
        Ok(())
    }
    /// Process in-place with deinterleaved channel pointers.
    pub fn process_f32(&mut self, inputs: &[&[f32]], outputs: &mut [&mut [f32]], nframes: usize) -> Result<()> {
        if !self.active { return Err(anyhow!("processor not active")); }
        // Build pointer arrays
        let in_ptrs: Vec<*const f32> = inputs.iter().map(|ch| ch.as_ptr()).collect();
        let mut out_ptrs: Vec<*mut f32> = outputs.iter_mut().map(|ch| ch.as_mut_ptr()).collect();
        let rc = unsafe { sys::v3_audio_processor_process_f32(
            self.proc, in_ptrs.as_ptr(), inputs.len() as i32,
            out_ptrs.as_mut_ptr(), outputs.len() as i32,
            nframes as i32) };
        if rc != 0 { return Err(anyhow!("process rc={rc}")); }
        Ok(())
    }
}
impl Drop for AudioProcessor {
    fn drop(&mut self) {
        unsafe {
            let _ = sys::v3_component_terminate(self.comp);
            let _ = sys::v3_release(self.proc as _);
            let _ = sys::v3_release(self.comp as _);
        }
    }
}
