use libloading::{Library, Symbol};
use std::path::{Path, PathBuf};
use thiserror::Error;

use openvst3_abi::{
    classinfo_consts, FactoryHandle, GetPluginFactoryProc, IPluginFactory, PClassInfo,
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
