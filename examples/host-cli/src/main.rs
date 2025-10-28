use clap::Parser;
use openvst3_host as host;
use std::path::PathBuf;

/// Load a VST3 inner binary (.dll/.so/.dylib) and print IPluginFactory::countClasses()
#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Path to inner binary (not the .vst3 directory). Phase 4 will add bundle resolution.
    #[arg(long, value_name = "FILE")]
    plugin: PathBuf,
}

fn main() {
    let args = Args::parse();
    match host::Module::load(&args.plugin) {
        Ok(mut module) => {
            let n = host::count_classes(&mut module);
            println!("countClasses = {n}");
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
