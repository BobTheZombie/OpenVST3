use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
struct Args {
    /// Path to the VST3 binary (.so inside the .vst3 bundle)
    #[arg(long)]
    plugin: PathBuf,
    /// Blocks to render
    #[arg(long, default_value_t = 16)]
    blocks: usize,
    /// Block size (frames)
    #[arg(long, default_value_t = 256)]
    block_size: usize,
    /// Sample rate
    #[arg(long, default_value_t = 48000)]
    sr: u32,
    /// Input channels
    #[arg(long, default_value_t = 2)]
    r#in: usize,
    /// Output channels
    #[arg(long, default_value_t = 2)]
    out: usize,
}

fn main() -> Result<()> {
    let a = Args::parse();
    unsafe {
        let f = openvst3_host::LoadedFactory::load_plugin(&a.plugin)?;
        let classes = f.classes()?;
        let cid = classes
            .first()
            .ok_or_else(|| anyhow::anyhow!("no classes in factory"))?
            .cid;
        println!("Using class: {} [{}]", classes[0].name, classes[0].category);
        let mut proc = f.create_audio_processor(cid)?;
        proc.initialize()?;
        proc.setup(
            a.sr as f64,
            a.block_size as i32,
            a.r#in as i32,
            a.out as i32,
        )?;
        proc.set_active(true)?;

        // Prepare buffers
        let mut inputs: Vec<Vec<f32>> = (0..a.r#in).map(|_| vec![0.0; a.block_size]).collect();
        let mut outputs: Vec<Vec<f32>> = (0..a.out).map(|_| vec![0.0; a.block_size]).collect();

        for _ in 0..a.blocks {
            let in_slices: Vec<&[f32]> = inputs.iter().map(|v| &v[..]).collect();
            let mut out_slices: Vec<&mut [f32]> = outputs.iter_mut().map(|v| &mut v[..]).collect();
            proc.process_f32(&in_slices, &mut out_slices, a.block_size)?;
        }
        proc.set_active(false)?;
    }
    println!("Done.");
    Ok(())
}
