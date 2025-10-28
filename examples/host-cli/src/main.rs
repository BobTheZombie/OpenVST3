use clap::Parser;
use openvst3_host as host;
use std::path::PathBuf;

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

    /// IID (16-byte hex) of interface to request at createInstance (e.g. IAudioProcessor IID)
    #[arg(long, value_name = "HEX16")]
    iid: Option<String>,

    /// Drive a single null 32f process block with N frames on OUTS channels (requires --class and --iid)
    #[arg(long, default_value_t = 0)]
    process_frames: i32,

    #[arg(long, default_value_t = 2)]
    process_outs: i32,

    #[arg(long, default_value_t = 48000.0)]
    sample_rate: f64,
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
            if let (Some(idx), Some(iid_hex)) = (args.class, args.iid.as_deref()) {
                let (_, _, cid_bytes) = match host::read_class_info_v1(&mut module, idx) {
                    Ok(x) => x,
                    Err(e) => {
                        eprintln!("class read error: {e}");
                        std::process::exit(4);
                    }
                };
                let iid = match host::parse_hex_16(iid_hex) {
                    Ok(x) => x,
                    Err(e) => {
                        eprintln!("iid parse error: {e}");
                        std::process::exit(5);
                    }
                };
                unsafe {
                    let raw = match host::create_instance_raw(module.factory_mut(), cid_bytes, iid)
                    {
                        Ok(p) => p,
                        Err(e) => {
                            eprintln!("createInstance error: {e}");
                            std::process::exit(6);
                        }
                    };
                    if args.process_frames > 0 {
                        match host::drive_null_process_32f(
                            raw,
                            args.sample_rate,
                            args.process_frames,
                            args.process_outs,
                        ) {
                            Ok(_) => println!(
                                "process() OK ({} frames, {} outs)",
                                args.process_frames, args.process_outs
                            ),
                            Err(e) => {
                                eprintln!("process error: {e}");
                                std::process::exit(7);
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
