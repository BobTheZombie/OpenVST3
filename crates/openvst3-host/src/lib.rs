use libloading::{Library, Symbol};
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::ptr::{self, NonNull};
use thiserror::Error;

use openvst3_abi::{
    classinfo_consts, process_consts, AudioBusBuffers32, FUnknown, FactoryHandle,
    GetPluginFactoryProc, IAudioProcessor, IPluginFactory, PClassInfo, ProcessData32, ProcessSetup,
    Tuid, K_RESULT_OK,
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
    #[error(
        "IID hex string must contain 32 hex characters (after removing separators), found {0}"
    )]
    HexLength(usize),
    #[error("invalid IID hex pair `{0}`")]
    HexParse(String),
    #[error("operation `{0}` failed with tresult {1}")]
    TResult(&'static str, i32),
    #[error("createInstance returned null object")]
    NullObject,
    #[error("audio processor pointer was null")]
    NullProcessor,
}

/// Handle for a loaded VST3 module binary (inner .dll/.dylib/.so)
pub struct Module {
    _lib: Library,
    factory: FactoryHandle,
}

impl Module {
    /// Load a platform binary (NOT the outer `.vst3` directory).
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, HostError> {
        let lib =
            unsafe { Library::new(path.as_ref()) }.map_err(|e| HostError::Dlopen(e.to_string()))?;
        let get_factory: Symbol<GetPluginFactoryProc> = unsafe {
            lib.get(b"GetPluginFactory\0")
                .map_err(|_| HostError::NoFactorySymbol)?
        };
        let raw = unsafe { get_factory() };
        let factory = unsafe { FactoryHandle::new(raw) }.ok_or(HostError::NullFactory)?;
        Ok(Self { _lib: lib, factory })
    }

    #[inline]
    pub fn factory_mut(&mut self) -> &mut IPluginFactory {
        self.factory.as_mut()
    }
}

unsafe impl Send for Module {}
unsafe impl Sync for Module {}

/// Utility: count classes via IPluginFactory
pub fn count_classes(module: &mut Module) -> i32 {
    unsafe { module.factory_mut().count_classes() }
}

/// BundlePath: resolve `.vst3` directory to inner binary per platform
pub struct BundlePath;

impl BundlePath {
    pub fn resolve<P: AsRef<Path>>(bundle: P) -> Result<PathBuf, HostError> {
        let b = bundle.as_ref();
        if !b.is_dir() || b.extension().and_then(|s| s.to_str()) != Some("vst3") {
            return Err(HostError::InvalidBundle(format!("{}", b.display())));
        }

        #[cfg(target_os = "macos")]
        {
            // My.vst3/Contents/MacOS/<binary>
            let p = b.join("Contents").join("MacOS");
            let bin = first_file(&p).ok_or(HostError::BinaryNotFound)?;
            return Ok(bin);
        }

        #[cfg(target_os = "linux")]
        {
            // My.vst3/Contents/x86_64-linux/<binary>.so (or aarch64-linux)
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
            // My.vst3/Contents/x86_64-win/My.vst3  (binary is named .vst3)
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

fn first_file(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok().and_then(|entries| {
        entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .find(|path| path.is_file())
    })
}

/// Convert a fixed-size i8 array (C string) to Rust String
fn cstr_from_i8_fixed(buf: &[i8]) -> Result<String, HostError> {
    // Find first NUL or end
    let mut bytes: Vec<u8> = Vec::with_capacity(buf.len());
    for &ch in buf {
        if ch == 0 {
            break;
        }
        bytes.push(ch as u8);
    }
    String::from_utf8(bytes).map_err(|_| HostError::Utf8)
}

/// Read class info at index using v1 API
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
    if tr != openvst3_abi::K_RESULT_OK {
        return Err(HostError::InvalidBundle(format!(
            "getClassInfo({index}) -> {tr}"
        )));
    }
    let name = cstr_from_i8_fixed(&info.name)?;
    let category = cstr_from_i8_fixed(&info.category)?;
    // copy CID bytes as u8
    let mut cid = [0u8; 16];
    for (i, b) in info.cid.iter().enumerate() {
        cid[i] = *b as u8;
    }
    Ok((name, category, cid))
}

/// Pretty-hex for CID
pub fn fmt_cid_hex(cid: &[u8; 16]) -> String {
    let mut s = String::with_capacity(32);
    for b in cid {
        s.push_str(&format!("{:02X}", b));
    }
    s
}

/// Scan and print all classes
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

/// Parse a 16-byte IID/CID from a hex string (accepts optional whitespace/hyphens)
pub fn parse_hex_16(input: &str) -> Result<Tuid, HostError> {
    let filtered: String = input
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '-')
        .collect();
    if filtered.len() != 32 {
        return Err(HostError::HexLength(filtered.len()));
    }
    let mut bytes = [0u8; 16];
    for (i, chunk) in filtered.as_bytes().chunks(2).enumerate() {
        let pair = match std::str::from_utf8(chunk) {
            Ok(p) => p,
            Err(_) => return Err(HostError::HexParse(format!("{:?}", chunk))),
        };
        let byte =
            u8::from_str_radix(pair, 16).map_err(|_| HostError::HexParse(pair.to_string()))?;
        bytes[i] = byte;
    }
    Ok(Tuid::new(bytes))
}

/// Unsafe helper: create raw instance pointer via IPluginFactory
pub unsafe fn create_instance_raw(
    factory: &mut IPluginFactory,
    cid_bytes: [u8; 16],
    iid: Tuid,
) -> Result<*mut c_void, HostError> {
    let mut obj: *mut c_void = ptr::null_mut();
    let cid = Tuid::new(cid_bytes);
    let tr = factory.create_instance_raw(&cid, &iid, &mut obj as *mut _);
    if tr != K_RESULT_OK {
        return Err(HostError::TResult("createInstance", tr));
    }
    if obj.is_null() {
        return Err(HostError::NullObject);
    }
    Ok(obj)
}

/// Drive a single 32-bit floating-point process block with silent buffers
pub unsafe fn drive_null_process_32f(
    instance: *mut c_void,
    sample_rate: f64,
    frames: i32,
    outs: i32,
) -> Result<(), HostError> {
    let mut proc =
        NonNull::new(instance as *mut IAudioProcessor).ok_or(HostError::NullProcessor)?;
    let proc = proc.as_mut();

    let mut need_terminate = false;
    let mut processing_on = false;

    let result = || -> Result<(), HostError> {
        let tr = proc.initialize(ptr::null_mut());
        if tr != K_RESULT_OK {
            return Err(HostError::TResult("IAudioProcessor::initialize", tr));
        }
        need_terminate = true;

        let frames_nonneg = frames.max(0);
        let outs_nonneg = outs.max(0);

        let setup = ProcessSetup {
            process_mode: process_consts::PROCESS_MODE_REALTIME,
            sample_rate,
            max_samples_per_block: frames_nonneg,
            symbolic_sample_size: process_consts::SYMBOLIC_SAMPLE_32,
            flags: 0,
        };
        let tr = proc.setup_processing(&setup);
        if tr != K_RESULT_OK {
            return Err(HostError::TResult("IAudioProcessor::setupProcessing", tr));
        }

        let frame_count = frames_nonneg as usize;
        let out_channels = outs_nonneg as usize;
        let mut channel_storage: Vec<Vec<openvst3_abi::Sample32>> =
            (0..out_channels).map(|_| vec![0.0; frame_count]).collect();
        let mut channel_ptrs: Vec<*mut openvst3_abi::Sample32> = channel_storage
            .iter_mut()
            .map(|buf| buf.as_mut_ptr())
            .collect();
        let mut out_bus = AudioBusBuffers32 {
            num_channels: outs_nonneg,
            silence_flags: if outs_nonneg <= 0 {
                0
            } else if outs_nonneg as u32 >= 64 {
                u64::MAX
            } else {
                (1u64 << outs_nonneg) - 1
            },
            channel_buffers: channel_ptrs.as_mut_ptr(),
        };
        let mut process_data = ProcessData32 {
            num_inputs: 0,
            num_outputs: 1,
            inputs: ptr::null_mut(),
            outputs: &mut out_bus as *mut _,
            num_samples: frames_nonneg,
        };

        let tr = proc.set_processing(1);
        if tr != K_RESULT_OK {
            return Err(HostError::TResult("IAudioProcessor::setProcessing(1)", tr));
        }
        processing_on = true;

        let tr = proc.process_32f(&mut process_data);
        if tr != K_RESULT_OK {
            return Err(HostError::TResult("IAudioProcessor::process", tr));
        }

        Ok(())
    }();

    if processing_on {
        let _ = proc.set_processing(0);
    }
    if need_terminate {
        let _ = proc.terminate();
    }

    let unknown = instance as *mut FUnknown;
    let _ = (*unknown).release();

    result
}
