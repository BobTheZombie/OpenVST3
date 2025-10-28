//! Minimal VST3 host loader: dlopen/LoadLibrary a .vst3 inner binary,
//! resolve `GetPluginFactory`, and drive IPluginFactory.
//! No Steinberg headers used.

use libloading::{Library, Symbol};
use thiserror::Error;
use openvst3_abi::{FactoryHandle, GetPluginFactoryProc, IPluginFactory};

#[derive(Debug, Error)]
pub enum HostError {
    #[error("dlopen failed: {0}")]
    Dlopen(String),
    #[error("symbol `GetPluginFactory` not found")]
    NoFactorySymbol,
    #[error("`GetPluginFactory` returned null")]
    NullFactory,
}

/// Opaque handle for a loaded VST3 module binary (inner .dll/.dylib/.so)
pub struct Module {
    _lib: Library,
    factory: FactoryHandle,
}

impl Module {
    /// Load a platform binary (NOT the outer `.vst3` directory). Phase 4 will add bundle discovery.
    pub fn load<P: AsRef<std::path::Path>>(path: P) -> Result<Self, HostError> {
        // Safety: libloading upholds safety; we only keep Library owned in this struct.
        let lib = unsafe { Library::new(path.as_ref()) }
            .map_err(|e| HostError::Dlopen(e.to_string()))?;

        // Safety: resolve symbol with C ABI
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

