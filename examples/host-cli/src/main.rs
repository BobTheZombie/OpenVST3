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

    /// If set, list all exported classes (index, category, name, CID)
    #[arg(long)]
    list: bool,
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
            if args.list {
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
            } else {
                let n = host::count_classes(&mut module);
                println!("countClasses = {n}");
            }
        }
        Err(e) => {
            eprintln!("load error: {e}");
            std::process::exit(1);
        }
    }
}
