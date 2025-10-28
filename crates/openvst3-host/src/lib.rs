use libloading::{Library, Symbol};
use std::path::{Path, PathBuf};
use thiserror::Error;

use openvst3_abi::{
    classinfo_consts, process_consts, AudioBusBuffers32, AudioBusBuffers64, BusInfo, FUnknown,
    FactoryHandle, GetPluginFactoryProc, IAudioProcessor, IComponent, IPluginFactory, PClassInfo,
    ProcessData32, ProcessData64, ProcessSetup, Tuid, BUS_DIR_OUTPUT, K_RESULT_OK,
};

#[derive(Debug, Error)]
pub enum HostError {
    #[error("dlopen failed: {0}")]
    Dlopen(String),
    #[error("symbol `GetPluginFactory` not found")]
    NoFactorySymbol,
    #[error("`GetPluginFactory` returned null")]
    NullFactory,
    #[error("not a valid VST3 bundle: {0}")]
    InvalidBundle(String),
    #[error("no platform binary found in bundle")]
    BinaryNotFound,
    #[error("utf8 error in class info")]
    Utf8,
    #[error("tresult failure: {0}")]
    TErr(i32),
    #[error("allocation")]
    Alloc,
    #[error("query interface failed")]
    NoInterface,
}

/// Handle for a loaded VST3 module binary
pub struct Module {
    lib: Library,
    factory: FactoryHandle,
}

impl Module {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, HostError> {
        let lib =
            unsafe { Library::new(path.as_ref()) }.map_err(|e| HostError::Dlopen(e.to_string()))?;
        let get_factory: Symbol<GetPluginFactoryProc> = unsafe {
            lib.get(b"GetPluginFactory\0")
                .map_err(|_| HostError::NoFactorySymbol)?
        };
        let raw = unsafe { get_factory() };
        let factory = unsafe { FactoryHandle::new(raw) }.ok_or(HostError::NullFactory)?;
        Ok(Self { lib, factory })
    }
    #[inline]
    pub fn factory_mut(&mut self) -> &mut IPluginFactory {
        self.factory.as_mut()
    }
}
unsafe impl Send for Module {}
unsafe impl Sync for Module {}

pub fn count_classes(module: &mut Module) -> i32 {
    unsafe { module.factory_mut().count_classes() }
}

/// BundlePath: resolve `.vst3` directory to inner binary per platform
pub struct BundlePath;

fn first_file(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.find_map(|e| {
        e.ok().and_then(|entry| {
            let path = entry.path();
            if path.is_file() {
                Some(path)
            } else {
                None
            }
        })
    })
}

impl BundlePath {
    pub fn resolve<P: AsRef<Path>>(bundle: P) -> Result<PathBuf, HostError> {
        let b = bundle.as_ref();
        if !b.is_dir() || b.extension().and_then(|s| s.to_str()) != Some("vst3") {
            return Err(HostError::InvalidBundle(format!("{}", b.display())));
        }
        #[cfg(target_os = "macos")]
        {
            let p = b.join("Contents").join("MacOS");
            let bin = first_file(&p).ok_or(HostError::BinaryNotFound)?;
            return Ok(bin);
        }
        #[cfg(target_os = "linux")]
        {
            let arch = if cfg!(target_arch = "x86_64") {
                "x86_64-linux"
            } else if cfg!(target_arch = "aarch64") {
                "aarch64-linux"
            } else {
                "unknown-linux"
            };
            let p = b.join("Contents").join(arch);
            let bin = first_file(&p).ok_or(HostError::BinaryNotFound)?;
            return Ok(bin);
        }
        #[cfg(target_os = "windows")]
        {
            let arch = if cfg!(target_arch = "x86_64") {
                "x86_64-win"
            } else {
                "x86-win"
            };
            let p = b.join("Contents").join(arch);
            let bin = first_file(&p).ok_or(HostError::BinaryNotFound)?;
            return Ok(bin);
        }
    }
}

// ----- Class info helpers (v1) -----------------------------------------------
fn cstr_from_i8_fixed(buf: &[i8]) -> Result<String, HostError> {
    let mut bytes: Vec<u8> = Vec::with_capacity(buf.len());
    for &ch in buf {
        if ch == 0 {
            break;
        }
        bytes.push(ch as u8);
    }
    String::from_utf8(bytes).map_err(|_| HostError::Utf8)
}

pub fn read_class_info_v1(
    module: &mut Module,
    index: i32,
) -> Result<(String, String, [u8; 16]), HostError> {
    let mut info = PClassInfo {
        cid: [0; 16],
        cardinality: 0,
        category: [0; classinfo_consts::K_CATEGORY_SIZE],
        name: [0; classinfo_consts::K_NAME_SIZE],
    };
    let tr = unsafe {
        module
            .factory_mut()
            .get_class_info(index, &mut info as *mut _)
    };
    if tr != K_RESULT_OK {
        return Err(HostError::TErr(tr));
    }
    let name = cstr_from_i8_fixed(&info.name)?;
    let category = cstr_from_i8_fixed(&info.category)?;
    let mut cid = [0u8; 16];
    for (i, b) in info.cid.iter().enumerate() {
        cid[i] = *b as u8;
    }
    Ok((name, category, cid))
}

pub fn fmt_cid_hex(cid: &[u8; 16]) -> String {
    let mut s = String::with_capacity(32);
    for b in cid {
        s.push_str(&format!("{:02X}", b));
    }
    s
}

pub fn list_classes(
    module: &mut Module,
) -> Result<Vec<(i32, String, String, [u8; 16])>, HostError> {
    let n = count_classes(module);
    let mut out = Vec::new();
    for i in 0..n {
        if let Ok((name, cat, cid)) = read_class_info_v1(module, i) {
            out.push((i, name, cat, cid));
        }
    }
    Ok(out)
}

// ===== Phase 4 helpers: IID parsing, QI, process 32f/64f =====================
pub fn parse_hex_16(s: &str) -> Result<[u8; 16], HostError> {
    let t = s
        .trim()
        .replace(|c: char| c == '-' || c == '{' || c == '}' || c == ' ', "");
    if t.len() != 32 {
        return Err(HostError::InvalidBundle(
            "IID hex must be 16 bytes (32 hex chars)".into(),
        ));
    }
    let mut out = [0u8; 16];
    for i in 0..16 {
        out[i] = u8::from_str_radix(&t[2 * i..2 * i + 2], 16)
            .map_err(|_| HostError::InvalidBundle("bad hex".into()))?;
    }
    Ok(out)
}

/// Create an instance using cid & iid (both raw 16-byte); returns raw void*.
pub unsafe fn create_instance_raw(
    factory: &mut IPluginFactory,
    cid: [u8; 16],
    iid: [u8; 16],
) -> Result<*mut core::ffi::c_void, HostError> {
    let mut obj: *mut core::ffi::c_void = core::ptr::null_mut();
    let tr = factory.create_instance_raw(&Tuid(cid), &Tuid(iid), &mut obj);
    if tr != K_RESULT_OK || obj.is_null() {
        return Err(HostError::TErr(tr));
    }
    Ok(obj)
}

/// Try QueryInterface on an object (FUnknown*) for a new IID; returns raw void*.
pub unsafe fn query_interface(
    obj: *mut core::ffi::c_void,
    iid: [u8; 16],
) -> Result<*mut core::ffi::c_void, HostError> {
    let fu: &mut FUnknown = &mut *(obj as *mut FUnknown);
    let mut out: *mut core::ffi::c_void = core::ptr::null_mut();
    let tr = fu.query_interface(&Tuid(iid), &mut out);
    if tr != K_RESULT_OK || out.is_null() {
        return Err(HostError::NoInterface);
    }
    Ok(out)
}

/// Read the first output bus channel count if available (graceful on failure)
pub unsafe fn detect_output_channels(comp_ptr: *mut IComponent) -> i32 {
    let comp = &mut *comp_ptr;
    let count = comp.get_bus_count(0, BUS_DIR_OUTPUT);
    if count <= 0 {
        return 2;
    }
    let mut info = BusInfo {
        media_type: 0,
        direction: BUS_DIR_OUTPUT,
        channel_count: 0,
        name: [0; 64],
        bus_type: 0,
        flags: 0,
    };
    let _ = comp.get_bus_info(0, BUS_DIR_OUTPUT, 0, &mut info as *mut _);
    if info.channel_count > 0 {
        info.channel_count
    } else {
        2
    }
}

/// Drive one 32f process block on an IAudioProcessor*
pub unsafe fn drive_null_process_32f(
    proc_ptr: *mut IAudioProcessor,
    sr: f64,
    nframes: i32,
    outs: i32,
) -> Result<(), HostError> {
    let proc = &mut *proc_ptr;

    // initialize(null)
    let tr = proc.initialize(core::ptr::null_mut::<FUnknown>());
    if tr != K_RESULT_OK {
        return Err(HostError::TErr(tr));
    }

    // setupProcessing
    let setup = ProcessSetup {
        process_mode: process_consts::PROCESS_MODE_REALTIME,
        sample_rate: sr,
        max_samples_per_block: nframes,
        symbolic_sample_size: process_consts::SYMBOLIC_SAMPLE_32,
        flags: 0,
    };
    let tr = proc.setup_processing(&setup);
    if tr != K_RESULT_OK {
        return Err(HostError::TErr(tr));
    }

    // allocate output buffers (silence)
    let mut chans: Vec<Vec<f32>> = (0..outs).map(|_| vec![0.0f32; nframes as usize]).collect();
    let mut chan_ptrs: Vec<*mut f32> = chans.iter_mut().map(|c| c.as_mut_ptr()).collect();
    let mut outs_bus = AudioBusBuffers32 {
        num_channels: outs,
        silence_flags: 0,
        channel_buffers: chan_ptrs.as_mut_ptr(),
    };

    let mut data = ProcessData32 {
        num_inputs: 0,
        num_outputs: 1,
        inputs: core::ptr::null_mut(),
        outputs: &mut outs_bus,
        num_samples: nframes,
    };

    let tr = proc.set_processing(1);
    if tr != K_RESULT_OK {
        return Err(HostError::TErr(tr));
    }

    let tr = proc.process_32f(&mut data);
    let _ = proc.set_processing(0);
    let _ = proc.terminate();

    if tr != K_RESULT_OK {
        return Err(HostError::TErr(tr));
    }
    Ok(())
}

/// Drive one 64f process block on an IAudioProcessor*
pub unsafe fn drive_null_process_64f(
    proc_ptr: *mut IAudioProcessor,
    sr: f64,
    nframes: i32,
    outs: i32,
) -> Result<(), HostError> {
    let proc = &mut *proc_ptr;

    let tr = proc.initialize(core::ptr::null_mut::<FUnknown>());
    if tr != K_RESULT_OK {
        return Err(HostError::TErr(tr));
    }

    let setup = ProcessSetup {
        process_mode: process_consts::PROCESS_MODE_REALTIME,
        sample_rate: sr,
        max_samples_per_block: nframes,
        symbolic_sample_size: process_consts::SYMBOLIC_SAMPLE_64,
        flags: 0,
    };
    let tr = proc.setup_processing(&setup);
    if tr != K_RESULT_OK {
        return Err(HostError::TErr(tr));
    }

    let mut chans: Vec<Vec<f64>> = (0..outs).map(|_| vec![0.0f64; nframes as usize]).collect();
    let mut chan_ptrs: Vec<*mut f64> = chans.iter_mut().map(|c| c.as_mut_ptr()).collect();
    let mut outs_bus = AudioBusBuffers64 {
        num_channels: outs,
        silence_flags: 0,
        channel_buffers: chan_ptrs.as_mut_ptr(),
    };

    let mut data = ProcessData64 {
        num_inputs: 0,
        num_outputs: 1,
        inputs: core::ptr::null_mut(),
        outputs: &mut outs_bus,
        num_samples: nframes,
    };

    let tr = proc.set_processing(1);
    if tr != K_RESULT_OK {
        return Err(HostError::TErr(tr));
    }

    let tr = proc.process_64f(&mut data);
    let _ = proc.set_processing(0);
    let _ = proc.terminate();

    if tr != K_RESULT_OK {
        return Err(HostError::TErr(tr));
    }
    Ok(())
}
