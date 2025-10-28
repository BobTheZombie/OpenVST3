use clap::Parser;
use openvst3_abi::IAudioProcessor;
use openvst3_host as host;
use std::path::PathBuf;

// Optional: load IIDs by name from iids.toml (same dir as binary or cwd)
fn load_iids() -> std::collections::BTreeMap<String, [u8; 16]> {
    let mut map = std::collections::BTreeMap::new();
    let candidates = [
        std::env::current_dir().unwrap().join("iids.toml"),
        std::env::current_exe().unwrap().with_file_name("iids.toml"),
    ];
    for p in candidates {
        if let Ok(s) = std::fs::read_to_string(&p) {
            for line in s.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                // format: Name = "HEX32"
                if let Some((k, v)) = line.split_once('=') {
                    let name = k.trim().trim_matches('"').trim_matches('\'').to_string();
                    let hex = v.trim().trim_matches('"').trim_matches('\'').to_string();
                    if let Ok(bytes) = host::parse_hex_16(&hex) {
                        map.insert(name, bytes);
                    }
                }
            }
            break;
        }
    }
    map
}

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Path to inner binary (.dll/.so/.dylib). Mutually exclusive with --bundle.
    #[arg(long, value_name = "FILE")]
    plugin: Option<PathBuf>,

    /// Path to a .vst3 bundle directory (we resolve the inner binary automatically)
    #[arg(long, value_name = "DIR")]
    bundle: Option<PathBuf>,

    /// List exported classes
    #[arg(long)]
    list: bool,

    /// Index of class to instantiate (from --list)
    #[arg(long)]
    class: Option<i32>,

    /// IID (16-byte hex) of interface to request at createInstance (e.g. IAudioProcessor)
    #[arg(long, value_name = "HEX32")]
    iid: Option<String>,

    /// IID name (looked up in iids.toml) if --iid is not provided; e.g., IAudioProcessor
    #[arg(long, value_name = "NAME")]
    iid_name: Option<String>,

    /// After instantiation, QueryInterface to this IID (hex or name) and drive that
    #[arg(long)]
    qi: bool,

    /// Drive a single null process block with N frames on OUTS channels (requires --class and --iid/--iid-name)
    #[arg(long, default_value_t = 0)]
    process_frames: i32,

    #[arg(long, default_value_t = 2)]
    process_outs: i32,

    #[arg(long, default_value_t = 48000.0)]
    sample_rate: f64,

    /// Use 64-bit float processing (default: 32-bit)
    #[arg(long)]
    float64: bool,
}

fn main() {
    let args = Args::parse();

    let bin = if let Some(p) = args.plugin {
        p
    } else if let Some(b) = args.bundle {
        match host::BundlePath::resolve(&b) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("bundle resolve error: {e}");
                std::process::exit(2);
            }
        }
    } else {
        eprintln!("Provide either --plugin <file> or --bundle <dir>");
        std::process::exit(2);
    };

    let iid_map = load_iids();

    match host::Module::load(&bin) {
        Ok(mut module) => {
            if args.list || args.class.is_none() {
                match host::list_classes(&mut module) {
                    Ok(list) => {
                        println!("classes = {}", list.len());
                        for (i, name, cat, cid) in list {
                            println!(
                                "#{i:02}  {:<22}  {:<24}  CID={}",
                                cat,
                                name,
                                host::fmt_cid_hex(&cid)
                            );
                        }
                    }
                    Err(e) => {
                        eprintln!("list error: {e}");
                        std::process::exit(3);
                    }
                }
            }
            if let Some(idx) = args.class {
                // grab class CID
                let (_, _, cid_bytes) = match host::read_class_info_v1(&mut module, idx) {
                    Ok(x) => x,
                    Err(e) => {
                        eprintln!("class read error: {e}");
                        std::process::exit(4);
                    }
                };

                // resolve IID
                let iid_bytes = if let Some(hex) = args.iid.as_deref() {
                    match host::parse_hex_16(hex) {
                        Ok(x) => x,
                        Err(e) => {
                            eprintln!("iid parse error: {e}");
                            std::process::exit(5);
                        }
                    }
                } else if let Some(name) = args.iid_name.as_deref() {
                    match iid_map.get(name) {
                        Some(b) => *b,
                        None => {
                            eprintln!("iid name not found in iids.toml: {}", name);
                            std::process::exit(5);
                        }
                    }
                } else {
                    eprintln!("provide --iid HEX32 or --iid-name from iids.toml");
                    std::process::exit(5);
                };

                unsafe {
                    // create instance
                    let created =
                        match host::create_instance_raw(module.factory_mut(), cid_bytes, iid_bytes)
                        {
                            Ok(p) => p,
                            Err(e) => {
                                eprintln!("createInstance error: {e}");
                                std::process::exit(6);
                            }
                        };

                    // if requested, QueryInterface to a different IID (by name or hex)
                    let target_ptr = if args.qi {
                        // if --iid-name was given, try the same; else use --iid again
                        let qi_iid = iid_bytes; // simple: QI to same IID; adjust if you pass another value
                        match host::query_interface(created, qi_iid) {
                            Ok(p) => p,
                            Err(e) => {
                                eprintln!("QI error: {e}");
                                std::process::exit(6);
                            }
                        }
                    } else {
                        created
                    };

                    if args.process_frames > 0 {
                        if args.float64 {
                            let proc_ptr = target_ptr as *mut IAudioProcessor;
                            match host::drive_null_process_64f(
                                proc_ptr,
                                args.sample_rate,
                                args.process_frames,
                                args.process_outs,
                            ) {
                                Ok(_) => println!(
                                    "process64() OK ({} frames, {} outs)",
                                    args.process_frames, args.process_outs
                                ),
                                Err(e) => {
                                    eprintln!("process64 error: {e}");
                                    std::process::exit(7);
                                }
                            }
                        } else {
                            let proc_ptr = target_ptr as *mut IAudioProcessor;
                            match host::drive_null_process_32f(
                                proc_ptr,
                                args.sample_rate,
                                args.process_frames,
                                args.process_outs,
                            ) {
                                Ok(_) => println!(
                                    "process32() OK ({} frames, {} outs)",
                                    args.process_frames, args.process_outs
                                ),
                                Err(e) => {
                                    eprintln!("process32 error: {e}");
                                    std::process::exit(7);
                                }
                            }
                        }
                    } else {
                        println!("Instance created (no processing requested).");
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("load error: {e}");
            std::process::exit(1);
        }
    }
}
