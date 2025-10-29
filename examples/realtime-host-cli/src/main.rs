use clap::Parser;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use openvst3_abi::{process_consts, IAudioProcessor, ProcessSetup};
use openvst3_host as host;
use std::path::PathBuf;

fn load_hex_iid(hex: &str) -> Result<[u8; 16], host::HostError> {
    host::parse_hex_16(hex)
}

fn parse_hex64_list(values: Option<&Vec<String>>) -> Result<Option<Vec<u64>>, host::HostError> {
    match values {
        Some(list) => {
            let mut out = Vec::new();
            for raw in list {
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let cleaned = trimmed.trim_start_matches("0x");
                let val = u64::from_str_radix(cleaned, 16).map_err(|_| {
                    host::HostError::InvalidBundle(format!("invalid hex64 value: {trimmed}"))
                })?;
                out.push(val);
            }
            if out.is_empty() {
                Ok(None)
            } else {
                Ok(Some(out))
            }
        }
        None => Ok(None),
    }
}

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Path to inner binary (.dll/.so/.dylib). Mutually exclusive with --bundle.
    #[arg(long, value_name = "FILE")]
    plugin: Option<PathBuf>,

    /// Path to a .vst3 bundle directory (resolve inner binary automatically).
    #[arg(long, value_name = "DIR")]
    bundle: Option<PathBuf>,

    /// Index of class to instantiate (from host-cli --list output).
    #[arg(long)]
    class: i32,

    /// IID (16-byte hex) of interface to request at createInstance (e.g. IAudioProcessor).
    #[arg(long, value_name = "HEX32")]
    iid: String,

    /// Optional IID (hex) to QueryInterface for IComponent (for bus info / diagnostics).
    #[arg(long, value_name = "HEX32")]
    component_iid: Option<String>,

    /// Maximum frames per callback (also requested from audio backend).
    #[arg(long, default_value_t = 512)]
    frames: u32,

    /// Request f64 stream processing (requires device support).
    #[arg(long)]
    float64: bool,

    /// Optional comma-separated input arrangement u64 IDs for setBusArrangements.
    #[arg(long, value_delimiter = ',')]
    in_arrs: Option<Vec<String>>,

    /// Optional comma-separated output arrangement u64 IDs for setBusArrangements.
    #[arg(long, value_delimiter = ',')]
    out_arrs: Option<Vec<String>>,
}

struct ProcessorRuntime {
    ptr: *mut IAudioProcessor,
    initialized: bool,
    processing: bool,
}

impl ProcessorRuntime {
    unsafe fn new(ptr: *mut IAudioProcessor) -> Self {
        Self {
            ptr,
            initialized: false,
            processing: false,
        }
    }

    fn ptr(&self) -> *mut IAudioProcessor {
        self.ptr
    }

    unsafe fn initialize(&mut self) -> Result<(), host::HostError> {
        if self.initialized {
            return Ok(());
        }
        let tr = (*self.ptr).initialize(core::ptr::null_mut());
        if tr != openvst3_abi::K_RESULT_OK {
            return Err(host::HostError::TErr(tr));
        }
        self.initialized = true;
        Ok(())
    }

    unsafe fn setup_processing(&mut self, setup: &ProcessSetup) -> Result<(), host::HostError> {
        let tr = (*self.ptr).setup_processing(setup);
        if tr != openvst3_abi::K_RESULT_OK {
            return Err(host::HostError::TErr(tr));
        }
        Ok(())
    }

    unsafe fn set_processing(&mut self, active: bool) -> Result<(), host::HostError> {
        let tr = (*self.ptr).set_processing(if active { 1 } else { 0 });
        if tr != openvst3_abi::K_RESULT_OK {
            return Err(host::HostError::TErr(tr));
        }
        self.processing = active;
        Ok(())
    }

    unsafe fn terminate(&mut self) -> Result<(), host::HostError> {
        if self.initialized {
            let tr = (*self.ptr).terminate();
            if tr != openvst3_abi::K_RESULT_OK {
                return Err(host::HostError::TErr(tr));
            }
            self.initialized = false;
        }
        Ok(())
    }
}

impl Drop for ProcessorRuntime {
    fn drop(&mut self) {
        unsafe {
            if self.ptr.is_null() {
                return;
            }
            if self.processing {
                let _ = (*self.ptr).set_processing(0);
                self.processing = false;
            }
            if self.initialized {
                let _ = (*self.ptr).terminate();
                self.initialized = false;
            }
            let base = self.ptr as *mut openvst3_abi::FUnknown;
            if !base.is_null() {
                let _ = (*base).release();
            }
        }
    }
}

struct CallbackState32 {
    proc_ptr: *mut IAudioProcessor,
    channels: usize,
    max_frames: usize,
    channel_data: Vec<Vec<f32>>,
    channel_ptrs: Vec<*mut f32>,
    outs_bus: openvst3_abi::AudioBusBuffers32,
}

impl CallbackState32 {
    unsafe fn new(proc_ptr: *mut IAudioProcessor, channels: usize, max_frames: usize) -> Self {
        let mut channel_data = Vec::with_capacity(channels);
        for _ in 0..channels {
            channel_data.push(vec![0.0f32; max_frames]);
        }
        let mut channel_ptrs = channel_data
            .iter_mut()
            .map(|c| c.as_mut_ptr())
            .collect::<Vec<_>>();
        let outs_bus = openvst3_abi::AudioBusBuffers32 {
            num_channels: channels as i32,
            silence_flags: 0,
            channel_buffers: channel_ptrs.as_mut_ptr(),
        };
        Self {
            proc_ptr,
            channels,
            max_frames,
            channel_data,
            channel_ptrs,
            outs_bus,
        }
    }

    unsafe fn process(&mut self, buffer: &mut [f32]) -> Result<(), host::HostError> {
        let frames = buffer.len() / self.channels;
        if frames > self.max_frames {
            return Err(host::HostError::InvalidBundle(format!(
                "callback frames ({frames}) exceed max block ({})",
                self.max_frames
            )));
        }
        for (idx, chan) in self.channel_data.iter_mut().enumerate() {
            self.channel_ptrs[idx] = chan.as_mut_ptr();
        }
        self.outs_bus.channel_buffers = self.channel_ptrs.as_mut_ptr();
        self.outs_bus.num_channels = self.channels as i32;
        self.outs_bus.silence_flags = 0;

        let mut data = openvst3_abi::ProcessData32 {
            num_inputs: 0,
            num_outputs: 1,
            inputs: core::ptr::null_mut(),
            outputs: &mut self.outs_bus,
            num_samples: frames as i32,
            input_parameter_changes: core::ptr::null_mut(),
            output_parameter_changes: core::ptr::null_mut(),
            input_events: core::ptr::null_mut(),
            output_events: core::ptr::null_mut(),
        };

        let proc = &mut *self.proc_ptr;
        let tr = proc.process_32f(&mut data);
        if tr != openvst3_abi::K_RESULT_OK {
            return Err(host::HostError::TErr(tr));
        }

        for frame in 0..frames {
            for ch in 0..self.channels {
                buffer[frame * self.channels + ch] = self.channel_data[ch][frame];
            }
        }
        Ok(())
    }
}

struct CallbackState64 {
    proc_ptr: *mut IAudioProcessor,
    channels: usize,
    max_frames: usize,
    channel_data: Vec<Vec<f64>>,
    channel_ptrs: Vec<*mut f64>,
    outs_bus: openvst3_abi::AudioBusBuffers64,
}

impl CallbackState64 {
    unsafe fn new(proc_ptr: *mut IAudioProcessor, channels: usize, max_frames: usize) -> Self {
        let mut channel_data = Vec::with_capacity(channels);
        for _ in 0..channels {
            channel_data.push(vec![0.0f64; max_frames]);
        }
        let mut channel_ptrs = channel_data
            .iter_mut()
            .map(|c| c.as_mut_ptr())
            .collect::<Vec<_>>();
        let outs_bus = openvst3_abi::AudioBusBuffers64 {
            num_channels: channels as i32,
            silence_flags: 0,
            channel_buffers: channel_ptrs.as_mut_ptr(),
        };
        Self {
            proc_ptr,
            channels,
            max_frames,
            channel_data,
            channel_ptrs,
            outs_bus,
        }
    }

    unsafe fn process(&mut self, buffer: &mut [f64]) -> Result<(), host::HostError> {
        let frames = buffer.len() / self.channels;
        if frames > self.max_frames {
            return Err(host::HostError::InvalidBundle(format!(
                "callback frames ({frames}) exceed max block ({})",
                self.max_frames
            )));
        }
        for (idx, chan) in self.channel_data.iter_mut().enumerate() {
            self.channel_ptrs[idx] = chan.as_mut_ptr();
        }
        self.outs_bus.channel_buffers = self.channel_ptrs.as_mut_ptr();
        self.outs_bus.num_channels = self.channels as i32;
        self.outs_bus.silence_flags = 0;

        let mut data = openvst3_abi::ProcessData64 {
            num_inputs: 0,
            num_outputs: 1,
            inputs: core::ptr::null_mut(),
            outputs: &mut self.outs_bus,
            num_samples: frames as i32,
            input_parameter_changes: core::ptr::null_mut(),
            output_parameter_changes: core::ptr::null_mut(),
            input_events: core::ptr::null_mut(),
            output_events: core::ptr::null_mut(),
        };

        let proc = &mut *self.proc_ptr;
        let tr = proc.process_64f(&mut data);
        if tr != openvst3_abi::K_RESULT_OK {
            return Err(host::HostError::TErr(tr));
        }

        for frame in 0..frames {
            for ch in 0..self.channels {
                buffer[frame * self.channels + ch] = self.channel_data[ch][frame];
            }
        }
        Ok(())
    }
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let bin = if let Some(p) = args.plugin {
        p
    } else if let Some(b) = args.bundle {
        host::BundlePath::resolve(&b).map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?
    } else {
        return Err("provide either --plugin <file> or --bundle <dir>".into());
    };

    let mut module =
        host::Module::load(&bin).map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
    let (_, _, cid) = host::read_class_info_v1(&mut module, args.class)
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
    let iid_bytes =
        load_hex_iid(&args.iid).map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;

    let created = unsafe {
        host::create_instance_raw(module.factory_mut(), cid, iid_bytes)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?
    };
    if created.is_null() {
        return Err("createInstance returned null".into());
    }
    let proc_ptr = created as *mut IAudioProcessor;
    if proc_ptr.is_null() {
        return Err("instance did not implement IAudioProcessor".into());
    }

    if let Some(hex) = args.component_iid.as_deref() {
        let comp_iid = load_hex_iid(hex).map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
        unsafe {
            if let Ok(ptr) = host::query_interface(created, comp_iid) {
                let outs = host::detect_output_channels(ptr as *mut openvst3_abi::IComponent);
                println!("component reports {outs} output channels (bus 0)");
            }
        }
    }

    let in_arrs = parse_hex64_list(args.in_arrs.as_ref())
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
    let out_arrs = parse_hex64_list(args.out_arrs.as_ref())
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;

    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "no default output device".to_string())?;

    let default_config = device.default_output_config()?;
    let config_to_use = if args.float64 {
        if default_config.sample_format() == cpal::SampleFormat::F64 {
            default_config
        } else {
            let mut found = None;
            for cfg in device.supported_output_configs()? {
                if cfg.sample_format() == cpal::SampleFormat::F64 {
                    found = Some(cfg.with_max_sample_rate());
                    break;
                }
            }
            found.ok_or_else(|| "no f64 output config available".to_string())?
        }
    } else {
        default_config
    };

    let sample_rate = config_to_use.sample_rate().0 as f64;
    let mut stream_config: cpal::StreamConfig = config_to_use.config();
    if args.frames == 0 {
        return Err("--frames must be > 0".into());
    }
    stream_config.buffer_size = cpal::BufferSize::Fixed(args.frames);
    let channels = stream_config.channels as usize;
    println!(
        "device: {} | sr: {} Hz | channels: {} | frames: {}",
        device.name()?,
        sample_rate,
        channels,
        args.frames
    );

    let mut runtime = unsafe { ProcessorRuntime::new(proc_ptr) };
    unsafe {
        runtime
            .initialize()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?
    };

    if in_arrs.is_some() || out_arrs.is_some() {
        let ins = in_arrs.as_deref().unwrap_or(&[]);
        let outs = out_arrs.as_deref().unwrap_or(&[]);
        unsafe {
            host::set_bus_arrangements(runtime.ptr(), ins, outs)
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
        }
    }

    let setup = ProcessSetup {
        process_mode: process_consts::PROCESS_MODE_REALTIME,
        sample_rate,
        max_samples_per_block: args.frames as i32,
        symbolic_sample_size: if matches!(config_to_use.sample_format(), cpal::SampleFormat::F64) {
            process_consts::SYMBOLIC_SAMPLE_64
        } else {
            process_consts::SYMBOLIC_SAMPLE_32
        },
        flags: 0,
    };
    unsafe {
        runtime
            .setup_processing(&setup)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
    }

    let err_fn = |err| eprintln!("stream error: {err}");

    let stream = match config_to_use.sample_format() {
        cpal::SampleFormat::F32 => {
            let mut state =
                unsafe { CallbackState32::new(runtime.ptr(), channels, args.frames as usize) };
            device.build_output_stream(
                &stream_config,
                move |data: &mut [f32], _| {
                    if let Err(e) = unsafe { state.process(data) } {
                        eprintln!("process32 error: {e}");
                    }
                },
                err_fn,
            )?
        }
        cpal::SampleFormat::F64 => {
            let mut state =
                unsafe { CallbackState64::new(runtime.ptr(), channels, args.frames as usize) };
            device.build_output_stream(
                &stream_config,
                move |data: &mut [f64], _| {
                    if let Err(e) = unsafe { state.process(data) } {
                        eprintln!("process64 error: {e}");
                    }
                },
                err_fn,
            )?
        }
        other => {
            return Err(format!("unsupported sample format: {other:?}").into());
        }
    };

    unsafe {
        runtime
            .set_processing(true)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
    }

    stream.play()?;
    println!("stream started. Press Enter to stop...");
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);

    unsafe {
        if let Err(e) = runtime.set_processing(false) {
            eprintln!("set_processing(false) error: {e}");
        }
        if let Err(e) = runtime.terminate() {
            eprintln!("terminate error: {e}");
        }
    }

    drop(stream);

    Ok(())
}
